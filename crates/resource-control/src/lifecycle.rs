mod adopt;
mod deletion;
mod promotion;

pub use adopt::add_remote;
pub use deletion::{DeletionMarker, deletion_markers};
pub use promotion::{PromotionResult, promote_projects};

use crate::canonical::{Selection, list_inventory, remove_canonical_resource};
use crate::observe::{cache_path, load_observation};
use crate::project::current_environment;
use crate::{SCHEMA_VERSION, git_root, load_project};
use anyhow::{Context, Result};
use deletion::MarkerScan;
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleResult {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub outcome: String,
}

pub fn remove(root: &Path, selection: &Selection) -> Result<Vec<LifecycleResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let inventory = list_inventory(&root, selection)?;
    let mut out = Vec::new();
    for item in inventory {
        let observation = load_observation(&cache_path(
            &root,
            &environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        ))?;
        let observed = observation
            .resources
            .get(&item.id)
            .context("Deletion Marker requires Observed State; run `taku fetch`")?;
        let guard = observed
            .guard
            .clone()
            .context("Deletion Marker requires a present observed identity guard")?;
        let source = root.join(&item.path);
        let marker_path = source.with_extension("delete.yaml");
        let marker = DeletionMarker {
            schema_version: SCHEMA_VERSION,
            environment: environment.clone(),
            target: item.target.clone(),
            namespace: item.namespace.clone(),
            resource_type: item.resource_type.clone(),
            id: item.id.clone(),
            parameters: item
                .value
                .as_object()
                .into_iter()
                .flat_map(|value| value.iter())
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect(),
            guard,
            source_path: item.path.clone(),
        };
        fs::write(&marker_path, serde_yaml::to_string(&marker)?)?;
        remove_canonical_resource(&source)?;
        out.push(LifecycleResult {
            environment: environment.clone(),
            target: item.target,
            namespace: item.namespace,
            resource_type: item.resource_type,
            id: item.id,
            outcome: "marked_for_deletion".into(),
        });
    }
    Ok(out)
}

pub fn forget(root: &Path, selection: &Selection) -> Result<Vec<LifecycleResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut out = Vec::new();
    for item in list_inventory(&root, selection)? {
        remove_canonical_resource(&root.join(&item.path))?;
        out.push(LifecycleResult {
            environment: environment.clone(),
            target: item.target,
            namespace: item.namespace,
            resource_type: item.resource_type,
            id: item.id,
            outcome: "forgotten".into(),
        });
    }
    let scan = MarkerScan {
        root: &root,
        project: &project,
        environment: &environment,
        selection,
    };
    scan.visit(|path, marker| {
        fs::remove_file(path)?;
        out.push(LifecycleResult {
            environment: marker.environment,
            target: marker.target,
            namespace: marker.namespace,
            resource_type: marker.resource_type,
            id: marker.id,
            outcome: "forgotten".into(),
        });
        Ok(())
    })?;
    Ok(out)
}

pub fn promote(
    root: &Path,
    from_environment: Option<&str>,
    to_environment: Option<&str>,
    from_target: Option<&str>,
    to_target: Option<&str>,
) -> Result<Vec<PromotionResult>> {
    promotion::promote(
        root,
        (from_environment, to_environment),
        (from_target, to_target),
    )
}
