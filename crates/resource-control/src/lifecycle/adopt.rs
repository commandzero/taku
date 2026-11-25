use super::LifecycleResult;
use crate::application::load_installed;
use crate::canonical::{
    Selection, resource_directory_in_namespace, resource_path, safe_filename,
    write_canonical_resource,
};
use crate::observe::{RemoteEntry, remote_list};
use crate::provider::resolve_auth;
use crate::resolution::discover;
use crate::transport::{OperationInput, RemoteResult, execute_retry_safe};
use crate::{Project, ResourceType, git_root, load_project};
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::path::Path;

pub fn add_remote(
    root: &Path,
    selection: &Selection,
    provider: &BTreeMap<String, String>,
) -> Result<Vec<LifecycleResult>> {
    if selection.ids.is_empty() {
        bail!("Add requires at least one exact Resource ID");
    }
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let remote = remote_list(&root, selection, false, provider)?;
    let mut out = Vec::new();
    for item in remote.into_iter().filter(|item| !item.tracked) {
        out.push(adopt(&root, &project, item, provider)?);
    }
    for id in &selection.ids {
        if !out.iter().any(|r| &r.id == id) {
            bail!("selected remote Resource {id} was not found or is already managed");
        }
    }
    Ok(out)
}

fn adopt(
    root: &Path,
    project: &Project,
    mut item: RemoteEntry,
    provider: &BTreeMap<String, String>,
) -> Result<LifecycleResult> {
    let target = &project.environments[&item.environment].targets[&item.target];
    let app = load_installed(root, &target.application)?;
    let auth = resolve_auth(root, project, &item.environment, target, provider)?;
    let discovered = discover(&app, target, &auth)?;
    let rt = &discovered.resource_types[&item.resource_type];
    let metadata_track = crate::hints::resolve(
        root,
        project,
        &item.environment,
        &item.target,
        item.namespace.as_deref(),
        &item.resource_type,
    )?
    .track;
    if let Some(read) = &rt.operations.read {
        let response = execute_retry_safe(
            target,
            &app,
            rt,
            read,
            OperationInput {
                namespace: item.namespace.as_deref(),
                id: Some(&item.id),
                context: Some(&item.value),
                metadata_track,
                ..Default::default()
            },
            &auth,
        )?;
        item.value = hydrated_resource(response, &item.id)?;
    }
    store_resource(root, project, &item, rt)?;
    Ok(LifecycleResult {
        environment: item.environment,
        target: item.target,
        namespace: item.namespace,
        resource_type: item.resource_type,
        id: item.id,
        outcome: "added".into(),
    })
}

fn store_resource(
    root: &Path,
    project: &Project,
    item: &RemoteEntry,
    rt: &ResourceType,
) -> Result<()> {
    let name = crate::canonical::display_name_value(&item.value, &rt.display_name)
        .unwrap_or_else(|| item.id.clone());
    let display = match rt.display_name.strategy {
        crate::DisplayNameStrategy::Id => item.id.clone(),
        crate::DisplayNameStrategy::Name => name,
        crate::DisplayNameStrategy::NameId => {
            format!("{}-{}", name, crate::canonical::short_id(&item.id))
        }
    };
    let directory = resource_directory_in_namespace(
        root,
        project,
        &item.environment,
        &item.target,
        item.namespace.as_deref(),
        &item.resource_type,
    );
    let filename = if rt.filesystem.is_some() {
        safe_filename(&display)
    } else {
        format!("{}.json", safe_filename(&display))
    };
    let path = resource_path(&directory, &item.value, rt, &filename)?;
    if path.exists() {
        bail!("Resource destination already exists: {}", path.display());
    }
    write_canonical_resource(&path, &item.value, rt)?;
    Ok(())
}

fn hydrated_resource(response: RemoteResult, id: &str) -> Result<serde_json::Value> {
    match response {
        RemoteResult::Success(mut values) if values.len() == 1 => Ok(values.pop().unwrap()),
        RemoteResult::NotFound => bail!("selected remote Resource {id} disappeared during Add"),
        RemoteResult::Conflict => bail!("selected remote Resource {id} conflicted during Add"),
        RemoteResult::Retryable | RemoteResult::Uncertain => {
            bail!("selected remote Resource {id} could not be read safely")
        }
        RemoteResult::Failure(message) => {
            bail!("selected remote Resource {id} could not be read: {message}")
        }
        RemoteResult::Success(_) => {
            bail!("selected remote Resource {id} did not hydrate to one Resource")
        }
    }
}
