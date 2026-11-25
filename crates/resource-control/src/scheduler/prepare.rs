mod deletion;
mod preflight;
mod resource;

pub(super) use preflight::preflight_transformations;

use super::{GroupKey, PreparedDeletion, PreparedResource, deletion_report, report};
use crate::canonical::InventoryEntry;
use crate::lifecycle::DeletionMarker;
use crate::observe::{Observation, ObservationFile, binding, cache_path, hash, load_observation};
use crate::provider::{SecretFields, resolve_auth};
use crate::reconcile::PushResult;
use crate::resolution::{ResolvedApplication, baseline_from, baseline_path, load_baseline};
use crate::{
    ApplicationDefinition, MissingPolicy, Project, ResourceType, TargetBaseline, TargetConfig,
};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::path::{Path, PathBuf};

pub(super) struct PushScope<'a> {
    pub(super) root: &'a Path,
    pub(super) project: &'a Project,
    pub(super) environment: &'a str,
    pub(super) cli_provider: &'a BTreeMap<String, String>,
    pub(super) missing: Option<MissingPolicy>,
}

impl PushScope<'_> {
    fn target(&self, name: &str) -> &TargetConfig {
        &self.project.environments[self.environment].targets[name]
    }

    fn auth(&self, target: &TargetConfig) -> Result<SecretFields> {
        resolve_auth(
            self.root,
            self.project,
            self.environment,
            target,
            self.cli_provider,
        )
    }

    fn checked_baseline(
        &self,
        location: &ResourceLocation<'_>,
        discovered: &ResolvedApplication,
    ) -> Result<TargetBaseline> {
        let baseline = load_required_baseline(self.root, self.environment, location.target)?;
        if baseline != baseline_from(discovered) {
            bail!(
                "Application Version selects different Resource Type Definitions; run `taku fetch` and `taku pull`"
            );
        }
        Ok(baseline)
    }
}

pub(super) struct PushPlan<'a> {
    environment: &'a str,
    binding_parts: Vec<String>,
    pub(super) resources: Vec<PreparedResource>,
    pub(super) deletions: Vec<PreparedDeletion>,
    pub(super) failed: BTreeSet<GroupKey>,
    pub(super) reports: Vec<PushResult>,
}

impl PushPlan<'_> {
    pub(super) fn binding(&mut self) -> String {
        self.binding_parts.sort();
        hash(self.binding_parts.join("\0").as_bytes())
    }

    pub(super) fn add_dry_run_reports(&mut self) {
        self.reports.extend(
            self.resources
                .iter()
                .map(|task| task.report(self.environment, "planned")),
        );
        self.reports.extend(self.deletions.iter().map(|task| {
            let outcome = if task.remote_absent {
                "planned_already_absent"
            } else {
                "planned_delete"
            };
            deletion_report(self.environment, &task.marker, outcome)
        }));
    }
}

pub(super) fn prepare<'a>(
    scope: &'a PushScope<'_>,
    inventory: Vec<InventoryEntry>,
    markers: Vec<(PathBuf, DeletionMarker)>,
    revision: &str,
) -> Result<PushPlan<'a>> {
    let mut prepare = Preparation {
        scope,
        observations: BTreeMap::new(),
        plan: PushPlan {
            environment: scope.environment,
            binding_parts: vec![serde_yaml::to_string(scope.project)?, revision.to_owned()],
            resources: Vec::new(),
            deletions: Vec::new(),
            failed: BTreeSet::new(),
            reports: Vec::new(),
        },
    };
    for item in inventory {
        prepare.resource(item)?;
    }
    for (path, marker) in markers {
        prepare.deletion(path, marker)?;
    }
    Ok(prepare.plan)
}

struct ResourceLocation<'a> {
    target: &'a str,
    namespace: Option<&'a str>,
    resource_type: &'a str,
    id: &'a str,
}

impl<'a> ResourceLocation<'a> {
    fn for_item(item: &'a InventoryEntry) -> Self {
        Self {
            target: &item.target,
            namespace: item.namespace.as_deref(),
            resource_type: &item.resource_type,
            id: &item.id,
        }
    }

    fn for_marker(marker: &'a DeletionMarker) -> Self {
        Self {
            target: &marker.target,
            namespace: marker.namespace.as_deref(),
            resource_type: &marker.resource_type,
            id: &marker.id,
        }
    }
}

struct Preparation<'a> {
    scope: &'a PushScope<'a>,
    plan: PushPlan<'a>,
    observations: BTreeMap<PathBuf, ObservationFile>,
}

impl Preparation<'_> {
    fn bind_hints(&mut self, location: &ResourceLocation<'_>) -> Result<bool> {
        let scope = self.scope;
        let hints = crate::hints::resolve(
            scope.root,
            scope.project,
            scope.environment,
            location.target,
            location.namespace,
            location.resource_type,
        )?;
        self.plan
            .binding_parts
            .push(hex::encode(hints.binding_material(scope.root)?));
        Ok(hints.track)
    }

    fn bind_definition(
        &mut self,
        app: &ApplicationDefinition,
        resource_type: &ResourceType,
        baseline: &TargetBaseline,
    ) -> Result<()> {
        self.plan.binding_parts.push(serde_yaml::to_string(app)?);
        self.plan
            .binding_parts
            .push(serde_yaml::to_string(resource_type)?);
        self.plan
            .binding_parts
            .push(serde_yaml::to_string(baseline)?);
        Ok(())
    }

    fn observation(
        &mut self,
        location: &ResourceLocation<'_>,
        app: &ApplicationDefinition,
        missing: &'static str,
    ) -> Result<(PathBuf, &Observation)> {
        let scope = self.scope;
        let path = cache_path(
            scope.root,
            scope.environment,
            location.target,
            location.namespace,
            location.resource_type,
        );
        let file = match self.observations.entry(path.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(load_observation(&path)?),
        };
        let expected = binding(
            scope.root,
            scope.project,
            scope.environment,
            location.target,
            app,
            location.namespace,
            location.resource_type,
        )?;
        if file.binding != expected {
            bail!("Observed State is structurally invalid; run `taku fetch`");
        }
        let resource = file.resources.get(location.id).context(missing)?;
        if resource.requires_pull {
            bail!("Observed State requires Pull after a configuration change; run `taku pull`");
        }
        Ok((path, resource))
    }
}

fn load_required_baseline(root: &Path, environment: &str, target: &str) -> Result<TargetBaseline> {
    load_baseline(&baseline_path(root, environment, target))
        .context("Target Baseline is unavailable; run `taku fetch`")
}
