use crate::application::load_installed;
use crate::canonical::{
    InventoryEntry, Selection, list_inventory, resource_directory_in_namespace, resource_path,
    write_canonical_resource,
};
use crate::project::current_environment;
use crate::resolution::{ResolvedApplication, baseline_path, for_local_use, load_baseline};
use crate::{IdScope, Project, RepositoryLayout, TargetConfig, git_root, load_project};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct PromotionResult {
    pub from_environment: String,
    pub to_environment: String,
    pub from_target: String,
    pub to_target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    pub id: String,
    pub outcome: String,
}

pub(super) fn promote(
    root: &Path,
    (from_environment, to_environment): (Option<&str>, Option<&str>),
    (from_target, to_target): (Option<&str>, Option<&str>),
) -> Result<Vec<PromotionResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    if project.layout != RepositoryLayout::Multi {
        bail!("Single-layout Promotion requires explicit source and destination Projects");
    }
    let destination = current_environment(&root, &project, to_environment)?;
    let source = from_environment
        .map(str::to_owned)
        .or_else(|| project.environments[&destination].from.clone())
        .context("destination Environment has no upstream `from` mapping")?;
    let targets = &project.environments[&destination].targets;
    let pairs = target_pairs(targets, from_target, to_target);
    let mut out = Vec::new();
    for (from, to) in pairs {
        let route = PromotionRoute {
            from_environment: &source,
            from_target: &from,
            to: Destination {
                root: &root,
                project: &project,
                environment: &destination,
                target: &to,
            },
        };
        route.promote_in_project(&mut out)?;
    }
    Ok(out)
}

fn target_pairs(
    targets: &BTreeMap<String, TargetConfig>,
    from_target: Option<&str>,
    to_target: Option<&str>,
) -> Vec<(String, String)> {
    if let Some(to) = to_target {
        let from = targets[to].from.as_deref().or(from_target).unwrap_or(to);
        return vec![(from.to_owned(), to.to_owned())];
    }
    targets
        .iter()
        .filter_map(|(to, target)| {
            let from = target.from.as_deref().unwrap_or(to);
            from_target
                .is_none_or(|filter| from == filter)
                .then(|| (from.to_owned(), to.clone()))
        })
        .collect()
}

pub fn promote_projects(
    source_root: &Path,
    destination_root: &Path,
    from_target: &str,
    to_target: &str,
) -> Result<Vec<PromotionResult>> {
    let source_root = git_root(source_root)?;
    let destination_root = git_root(destination_root)?;
    let source_project = load_project(&source_root)?;
    let destination_project = load_project(&destination_root)?;
    if (source_project.layout, destination_project.layout)
        != (RepositoryLayout::Single, RepositoryLayout::Single)
    {
        bail!("explicit cross-Project Promotion requires two Single-layout Projects");
    }
    let source_environment = source_project.environments.keys().next().unwrap().clone();
    let destination_environment = destination_project
        .environments
        .keys()
        .next()
        .unwrap()
        .clone();
    let source = Destination {
        root: &source_root,
        project: &source_project,
        environment: &source_environment,
        target: from_target,
    };
    let route = PromotionRoute {
        from_environment: &source_environment,
        from_target,
        to: Destination {
            root: &destination_root,
            project: &destination_project,
            environment: &destination_environment,
            target: to_target,
        },
    };
    let destination_resolved = cross_project_types(&source, &route.to)?;
    let selection = Selection {
        targets: vec![from_target.into()],
        ..Default::default()
    };
    list_inventory(&source_root, &selection)?
        .into_iter()
        .map(|item| route.copy(item, &destination_resolved))
        .collect()
}

fn cross_project_types(
    source: &Destination<'_>,
    destination: &Destination<'_>,
) -> Result<ResolvedApplication> {
    let source_target = &source.project.environments[source.environment].targets[source.target];
    let destination_target =
        &destination.project.environments[destination.environment].targets[destination.target];
    if source_target.application != destination_target.application {
        bail!("Promotion Targets use incompatible Applications");
    }
    let source_app = load_installed(source.root, &source_target.application)?;
    let destination_app = load_installed(destination.root, &destination_target.application)?;
    let source_baseline = load_baseline(&baseline_path(
        source.root,
        source.environment,
        source.target,
    ))
    .ok();
    let destination_baseline = load_baseline(&baseline_path(
        destination.root,
        destination.environment,
        destination.target,
    ))
    .ok();
    let source_resolved = for_local_use(&source_app, source_target, source_baseline.as_ref())?;
    let destination_resolved = for_local_use(
        &destination_app,
        destination_target,
        destination_baseline.as_ref(),
    )?;
    if resolved_identity(&source_resolved) != resolved_identity(&destination_resolved) {
        bail!("Promotion installed Applications are not exactly compatible");
    }
    Ok(destination_resolved)
}

fn resolved_identity(application: &ResolvedApplication) -> (&str, &str, &BTreeMap<String, String>) {
    (
        &application.application_version,
        &application.catalog_version,
        &application.selected,
    )
}

struct Destination<'a> {
    root: &'a Path,
    project: &'a Project,
    environment: &'a str,
    target: &'a str,
}

struct PromotionRoute<'a> {
    from_environment: &'a str,
    from_target: &'a str,
    to: Destination<'a>,
}

impl PromotionRoute<'_> {
    fn promote_in_project(&self, out: &mut Vec<PromotionResult>) -> Result<()> {
        let Some(source) = self.to.project.environments[self.from_environment]
            .targets
            .get(self.from_target)
        else {
            out.push(self.result(None, "skipped_unresolved"));
            return Ok(());
        };
        let destination =
            &self.to.project.environments[self.to.environment].targets[self.to.target];
        if source.application != destination.application {
            bail!(
                "Promotion Targets {} and {} use incompatible Applications",
                self.from_target,
                self.to.target
            );
        }
        let application = load_installed(self.to.root, &destination.application)?;
        let baseline = load_baseline(&baseline_path(
            self.to.root,
            self.to.environment,
            self.to.target,
        ))
        .ok();
        let resolved = for_local_use(&application, destination, baseline.as_ref())?;
        let selection = Selection {
            environment: Some(self.from_environment.into()),
            targets: vec![self.from_target.into()],
            ..Default::default()
        };
        for item in list_inventory(self.to.root, &selection)? {
            out.push(self.copy(item, &resolved)?);
        }
        Ok(())
    }

    fn copy(
        &self,
        item: InventoryEntry,
        resolved: &ResolvedApplication,
    ) -> Result<PromotionResult> {
        if item.id_scope != IdScope::Universal {
            bail!(
                "Resource {} has Target-scoped identity and cannot be promoted",
                item.id
            );
        }
        let filename = Path::new(&item.path)
            .file_name()
            .and_then(|name| name.to_str())
            .context("Resource filename is not valid UTF-8")?;
        let namespace = item.namespace.as_deref();
        let directory = resource_directory_in_namespace(
            self.to.root,
            self.to.project,
            self.to.environment,
            self.to.target,
            namespace,
            &item.resource_type,
        );
        let resource_type = &resolved.resource_types[&item.resource_type];
        let value = self.to.normalize(&item, resource_type)?;
        let path = resource_path(&directory, &value, resource_type, filename)?;
        write_canonical_resource(&path, &value, resource_type)?;
        Ok(self.result(Some(item), "promoted"))
    }

    fn result(&self, item: Option<InventoryEntry>, outcome: &str) -> PromotionResult {
        let (namespace, resource_type, id) = match item {
            Some(item) => (item.namespace, Some(item.resource_type), item.id),
            None => (None, None, String::new()),
        };
        PromotionResult {
            from_environment: self.from_environment.into(),
            to_environment: self.to.environment.into(),
            from_target: self.from_target.into(),
            to_target: self.to.target.into(),
            namespace,
            resource_type,
            id,
            outcome: outcome.into(),
        }
    }
}

impl Destination<'_> {
    fn normalize(
        &self,
        item: &InventoryEntry,
        resource_type: &crate::ResourceType,
    ) -> Result<serde_json::Value> {
        let hints = crate::hints::resolve(
            self.root,
            self.project,
            self.environment,
            self.target,
            item.namespace.as_deref(),
            &item.resource_type,
        )?;
        crate::hints::validate_tracking(resource_type, &hints, &item.resource_type)?;
        if hints.track {
            Ok(item.value.clone())
        } else {
            crate::transport::without_metadata(&item.value, resource_type)
        }
    }
}
