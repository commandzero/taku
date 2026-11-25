use super::{PullResult, check_binding, observation_at};
use crate::application::load_installed;
use crate::canonical::{InventoryEntry, Selection, list_inventory};
use crate::lifecycle::{DeletionMarker, deletion_markers};
use crate::observe::{ObservationFile, binding, cache_path, save_observation};
use crate::project::current_environment;
use crate::resolution::{
    ResolvedApplication, baseline_from, baseline_path, from_baseline, load_baseline, save_baseline,
};
use crate::{
    ApplicationDefinition, MissingPolicy, Project, TargetBaseline, git_root, load_project,
};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod decisions;
mod mutations;

use decisions::{PullPolicy, ResourceOutcome, reconcile_resource};
use mutations::{ResourceMutation, apply};

pub fn pull(
    root: &Path,
    selection: &Selection,
    missing: Option<MissingPolicy>,
) -> Result<Vec<PullResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let context = PullContext {
        root: &root,
        project: &project,
        environment: &environment,
    };
    let mut plan = PullPlan::default();
    for item in list_inventory(&root, selection)? {
        plan.resource(&context, item, missing)?;
    }
    for (path, marker) in deletion_markers(&root, selection)? {
        plan.marker(&context, path, marker)?;
    }
    plan.finish()
}

struct PullContext<'a> {
    root: &'a Path,
    project: &'a Project,
    environment: &'a str,
}

impl PullContext<'_> {
    fn observe<'a>(
        &self,
        observations: &'a mut BTreeMap<PathBuf, ObservationFile>,
        application: &ApplicationDefinition,
        (target, namespace, resource_type): (&str, Option<&str>, &str),
    ) -> Result<&'a mut ObservationFile> {
        let path = cache_path(
            self.root,
            self.environment,
            target,
            namespace,
            resource_type,
        );
        let observation = observation_at(observations, &path)?;
        let expected = binding(
            self.root,
            self.project,
            self.environment,
            target,
            application,
            namespace,
            resource_type,
        )?;
        check_binding(observation, &expected)?;
        Ok(observation)
    }
}

#[derive(Default)]
struct PullPlan {
    reports: Vec<PullResult>,
    observations: BTreeMap<PathBuf, ObservationFile>,
    resources: Vec<ResourceMutation>,
    markers: Vec<(PathBuf, DeletionMarker)>,
    baselines: BTreeMap<PathBuf, TargetBaseline>,
    has_conflict: bool,
}

impl PullPlan {
    fn resource(
        &mut self,
        context: &PullContext<'_>,
        item: InventoryEntry,
        missing: Option<MissingPolicy>,
    ) -> Result<()> {
        let target = &context.project.environments[context.environment].targets[&item.target];
        let application = load_installed(context.root, &target.application)?;
        let baseline = load_baseline(&baseline_path(
            context.root,
            context.environment,
            &item.target,
        ))?;
        let resolved = from_baseline(&application, target, &baseline)?;
        let resource_type = &resolved.resource_types[&item.resource_type];
        let observation = context.observe(
            &mut self.observations,
            &application,
            (&item.target, item.namespace.as_deref(), &item.resource_type),
        )?;
        let resource = observation
            .resources
            .get_mut(&item.id)
            .context("selected Resource is absent from Observed State")?;
        let metadata_track = crate::hints::resolve(
            context.root,
            context.project,
            context.environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        )?
        .track;
        let outcome = reconcile_resource(
            &item.value,
            resource_type,
            resource,
            PullPolicy {
                missing,
                metadata_track,
            },
        )?;
        match &outcome {
            ResourceOutcome::Pull(value) => self.resources.push((
                context.root.join(&item.path),
                Some(value.clone()),
                resource_type.clone(),
            )),
            ResourceOutcome::Delete => {
                self.resources
                    .push((context.root.join(&item.path), None, resource_type.clone()))
            }
            ResourceOutcome::Conflict(_) => self.has_conflict = true,
            ResourceOutcome::Unchanged | ResourceOutcome::LocalOnly => {}
        }
        if !matches!(outcome, ResourceOutcome::Conflict(_)) {
            resource.requires_pull = false;
            self.baselines.insert(
                baseline_path(context.root, context.environment, &item.target),
                observed_baseline(observation),
            );
        }
        self.reports.push(report(
            context.environment,
            (item.target, item.namespace, item.resource_type, item.id),
            outcome.report(),
        ));
        Ok(())
    }

    fn marker(
        &mut self,
        context: &PullContext<'_>,
        marker_path: PathBuf,
        mut marker: DeletionMarker,
    ) -> Result<()> {
        let target = &context.project.environments[context.environment].targets[&marker.target];
        let application = load_installed(context.root, &target.application)?;
        let observation = context.observe(
            &mut self.observations,
            &application,
            (
                &marker.target,
                marker.namespace.as_deref(),
                &marker.resource_type,
            ),
        )?;
        let resource = observation
            .resources
            .get_mut(&marker.id)
            .context("selected deletion marker is absent from Observed State")?;
        let mut outcome = "unchanged";
        if let Some(guard) = &resource.guard
            && &marker.guard != guard
        {
            marker.guard.clone_from(guard);
            self.markers.push((marker_path, marker.clone()));
            outcome = "pulled";
        }
        resource.requires_pull = false;
        self.baselines.insert(
            baseline_path(context.root, context.environment, &marker.target),
            observed_baseline(observation),
        );
        self.reports.push(report(
            context.environment,
            (
                marker.target,
                marker.namespace,
                marker.resource_type,
                marker.id,
            ),
            outcome,
        ));
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<PullResult>> {
        if self.has_conflict {
            for report in self
                .reports
                .iter_mut()
                .filter(|report| matches!(report.outcome.as_str(), "pulled" | "deleted"))
            {
                report.outcome = "blocked_by_conflict".into();
            }
            return Ok(self.reports);
        }
        apply(&self.resources)?;
        for (path, marker) in self.markers {
            std::fs::write(path, serde_yaml::to_string(&marker)?)?;
        }
        for (path, observation) in self.observations {
            save_observation(&path, &observation)?;
        }
        for (path, baseline) in self.baselines {
            save_baseline(&path, &baseline)?;
        }
        Ok(self.reports)
    }
}

fn observed_baseline(observation: &ObservationFile) -> TargetBaseline {
    baseline_from(&ResolvedApplication {
        application_version: observation.application_version.clone(),
        catalog_version: observation.catalog_version.clone(),
        selected: observation.definitions.clone(),
        resource_types: BTreeMap::new(),
    })
}

fn report(
    environment: &str,
    (target, namespace, resource_type, id): (String, Option<String>, String, String),
    outcome: &str,
) -> PullResult {
    PullResult {
        environment: environment.to_owned(),
        target,
        namespace,
        resource_type,
        id,
        outcome: outcome.into(),
    }
}
