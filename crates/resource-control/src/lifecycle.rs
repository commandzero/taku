use crate::application::load_installed;
use crate::canonical::{
    Selection, list_inventory, reject_symlink_components, remove_canonical_resource,
    resource_directories, resource_directory_in_namespace, safe_filename, write_canonical_resource,
};
use crate::observe::{cache_path, load_observation, remote_list};
use crate::project::current_environment;
use crate::provider::resolve_auth;
use crate::resolution::{baseline_path, discover, for_local_use, load_baseline};
use crate::transport::{OperationInput, RemoteResult, execute_retry_safe};
use crate::{IdScope, RepositoryLayout, SCHEMA_VERSION, git_root, load_project};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionMarker {
    pub schema_version: u32,
    pub environment: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
    pub guard: String,
    pub source_path: String,
}

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
    for mut item in remote {
        if item.tracked {
            continue;
        }
        let target = &project.environments[&item.environment].targets[&item.target];
        let app = load_installed(&root, &target.application)?;
        let auth = resolve_auth(&root, &project, &item.environment, target, provider)?;
        let discovered = discover(&app, target, &auth)?;
        let rt = &discovered.resource_types[&item.resource_type];
        let metadata_track = crate::hints::resolve(
            &root,
            &project,
            &item.environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        )?
        .track;
        if let Some(read) = &rt.operations.read {
            item.value = match execute_retry_safe(
                target,
                &app,
                rt,
                read,
                OperationInput {
                    namespace: item.namespace.as_deref(),
                    id: Some(&item.id),
                    context: Some(&item.value),
                    body: None,
                    resource_ids: None,
                    mutation: false,
                    metadata_track,
                },
                &auth,
            )? {
                RemoteResult::Success(mut values) if values.len() == 1 => values.pop().unwrap(),
                RemoteResult::NotFound => {
                    bail!(
                        "selected remote Resource {} disappeared during Add",
                        item.id
                    )
                }
                RemoteResult::Conflict => {
                    bail!("selected remote Resource {} conflicted during Add", item.id)
                }
                RemoteResult::Retryable | RemoteResult::Uncertain => {
                    bail!(
                        "selected remote Resource {} could not be read safely",
                        item.id
                    )
                }
                RemoteResult::Failure(message) => {
                    bail!(
                        "selected remote Resource {} could not be read: {message}",
                        item.id
                    )
                }
                RemoteResult::Success(_) => {
                    bail!(
                        "selected remote Resource {} did not hydrate to one Resource",
                        item.id
                    )
                }
            };
        }
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
            &root,
            &project,
            &item.environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        );
        let path = if rt.filesystem.is_some() {
            directory.join(safe_filename(&display))
        } else {
            directory.join(format!("{}.json", safe_filename(&display)))
        };
        if path.exists() {
            bail!("Resource destination already exists: {}", path.display());
        }
        write_canonical_resource(&path, &item.value, rt)?;
        out.push(LifecycleResult {
            environment: item.environment,
            target: item.target,
            namespace: item.namespace,
            resource_type: item.resource_type,
            id: item.id,
            outcome: "added".into(),
        });
    }
    for id in &selection.ids {
        if !out.iter().any(|r| &r.id == id) {
            bail!("selected remote Resource {id} was not found or is already managed");
        }
    }
    Ok(out)
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
    for (target_name, target) in &project.environments[&environment].targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        let baseline = load_baseline(&baseline_path(&root, &environment, target_name)).ok();
        let resolved = for_local_use(&app, target, baseline.as_ref())?;
        for (type_name, resource_type) in &resolved.resource_types {
            if !selection.types.is_empty() && !selection.types.contains(type_name) {
                continue;
            }
            for (namespace, dir) in resource_directories(
                &root,
                &project,
                &environment,
                target_name,
                type_name,
                resource_type.namespaced,
            )? {
                if !selection.namespaces.is_empty()
                    && namespace
                        .as_ref()
                        .is_none_or(|value| !selection.namespaces.contains(value))
                {
                    continue;
                }
                reject_symlink_components(&root, &dir)?;
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries {
                        let path = entry?.path();
                        if !path
                            .file_name()
                            .and_then(|v| v.to_str())
                            .is_some_and(|v| v.ends_with(".delete.yaml"))
                        {
                            continue;
                        }
                        let marker = load_deletion_marker(
                            &root,
                            &path,
                            &environment,
                            target_name,
                            namespace.as_deref(),
                            type_name,
                        )?;
                        if !selection.ids.is_empty() && !selection.ids.contains(&marker.id) {
                            continue;
                        }
                        fs::remove_file(path)?;
                        out.push(LifecycleResult {
                            environment: environment.clone(),
                            target: target_name.clone(),
                            namespace: namespace.clone(),
                            resource_type: type_name.clone(),
                            id: marker.id,
                            outcome: "forgotten".into(),
                        });
                    }
                }
            }
        }
    }
    Ok(out)
}

pub fn deletion_markers(
    root: &Path,
    selection: &Selection,
) -> Result<Vec<(std::path::PathBuf, DeletionMarker)>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut out = Vec::new();
    for (target_name, target) in &project.environments[&environment].targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        let baseline = load_baseline(&baseline_path(&root, &environment, target_name)).ok();
        let resolved = for_local_use(&app, target, baseline.as_ref())?;
        for (type_name, resource_type) in &resolved.resource_types {
            if !selection.types.is_empty() && !selection.types.contains(type_name) {
                continue;
            }
            for (namespace, dir) in resource_directories(
                &root,
                &project,
                &environment,
                target_name,
                type_name,
                resource_type.namespaced,
            )? {
                if !selection.namespaces.is_empty()
                    && namespace
                        .as_ref()
                        .is_none_or(|value| !selection.namespaces.contains(value))
                {
                    continue;
                }
                reject_symlink_components(&root, &dir)?;
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries {
                        let path = entry?.path();
                        if path
                            .file_name()
                            .and_then(|v| v.to_str())
                            .is_some_and(|v| v.ends_with(".delete.yaml"))
                        {
                            let marker = load_deletion_marker(
                                &root,
                                &path,
                                &environment,
                                target_name,
                                namespace.as_deref(),
                                type_name,
                            )?;
                            if selection.ids.is_empty() || selection.ids.contains(&marker.id) {
                                out.push((path, marker));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn load_deletion_marker(
    root: &Path,
    path: &Path,
    environment: &str,
    target: &str,
    namespace: Option<&str>,
    resource_type: &str,
) -> Result<DeletionMarker> {
    reject_symlink_components(root, path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "symlinked Deletion Marker input is not allowed: {}",
            path.display()
        );
    }
    if !metadata.is_file() {
        bail!("Deletion Marker is not a regular file: {}", path.display());
    }
    let marker: DeletionMarker = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    if marker.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Deletion Marker schema version {}",
            marker.schema_version
        );
    }
    if marker.environment != environment
        || marker.target != target
        || marker.namespace.as_deref() != namespace
        || marker.resource_type != resource_type
    {
        bail!("Deletion Marker binding does not match its Environment/Target/Resource Type tree");
    }
    let source = Path::new(&marker.source_path);
    if source.is_absolute() {
        bail!("Deletion Marker source_path must be Project-relative");
    }
    let expected = root.join(source).with_extension("delete.yaml");
    reject_symlink_components(root, &root.join(source))?;
    if expected != path {
        bail!("Deletion Marker source_path does not match its marker path");
    }
    Ok(marker)
}

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

struct DestinationScope<'a> {
    root: &'a Path,
    project: &'a crate::Project,
    environment: &'a str,
    target: &'a str,
    namespace: Option<&'a str>,
    type_name: &'a str,
}

fn normalize_for_destination(
    scope: DestinationScope<'_>,
    resource_type: &crate::ResourceType,
    value: &serde_json::Value,
) -> Result<serde_json::Value> {
    let hints = crate::hints::resolve(
        scope.root,
        scope.project,
        scope.environment,
        scope.target,
        scope.namespace,
        scope.type_name,
    )?;
    crate::hints::validate_tracking(resource_type, &hints, scope.type_name)?;
    if hints.track {
        Ok(value.clone())
    } else {
        crate::transport::without_metadata(value, resource_type)
    }
}

pub fn promote(
    root: &Path,
    from_environment: Option<&str>,
    to_environment: Option<&str>,
    from_target: Option<&str>,
    to_target: Option<&str>,
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
    let mut pairs = Vec::new();
    if let Some(to) = to_target {
        let target = &project.environments[&destination].targets[to];
        pairs.push((
            target
                .from
                .clone()
                .or_else(|| from_target.map(str::to_owned))
                .unwrap_or_else(|| to.into()),
            to.to_owned(),
        ));
    } else {
        for (to, target) in &project.environments[&destination].targets {
            if let Some(filter) = from_target
                && target.from.as_deref().unwrap_or(to) != filter
            {
                continue;
            }
            pairs.push((
                target.from.clone().unwrap_or_else(|| to.clone()),
                to.clone(),
            ));
        }
    }
    let mut out = Vec::new();
    for (from, to) in pairs {
        let Some(source_target) = project.environments[&source].targets.get(&from) else {
            out.push(PromotionResult {
                from_environment: source.clone(),
                to_environment: destination.clone(),
                from_target: from,
                to_target: to,
                namespace: None,
                resource_type: None,
                id: "".into(),
                outcome: "skipped_unresolved".into(),
            });
            continue;
        };
        let destination_target = &project.environments[&destination].targets[&to];
        if source_target.application != destination_target.application {
            bail!("Promotion Targets {from} and {to} use incompatible Applications");
        }
        let application = load_installed(&root, &destination_target.application)?;
        let destination_baseline = load_baseline(&baseline_path(&root, &destination, &to)).ok();
        let destination_application = for_local_use(
            &application,
            destination_target,
            destination_baseline.as_ref(),
        )?;
        let selection = Selection {
            environment: Some(source.clone()),
            targets: vec![from.clone()],
            namespaces: vec![],
            types: vec![],
            ids: vec![],
        };
        for item in list_inventory(&root, &selection)? {
            if item.id_scope != IdScope::Universal {
                bail!(
                    "Resource {} has Target-scoped identity and cannot be promoted",
                    item.id
                );
            }
            let source_path = Path::new(&item.path);
            let filename = source_path.file_name().unwrap();
            let destination_path = resource_directory_in_namespace(
                &root,
                &project,
                &destination,
                &to,
                item.namespace.as_deref(),
                &item.resource_type,
            )
            .join(filename);
            let resource_type = &destination_application.resource_types[&item.resource_type];
            let value = normalize_for_destination(
                DestinationScope {
                    root: &root,
                    project: &project,
                    environment: &destination,
                    target: &to,
                    namespace: item.namespace.as_deref(),
                    type_name: &item.resource_type,
                },
                resource_type,
                &item.value,
            )?;
            write_canonical_resource(&destination_path, &value, resource_type)?;
            out.push(PromotionResult {
                from_environment: source.clone(),
                to_environment: destination.clone(),
                from_target: from.clone(),
                to_target: to.clone(),
                namespace: item.namespace,
                resource_type: Some(item.resource_type),
                id: item.id,
                outcome: "promoted".into(),
            });
        }
    }
    Ok(out)
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
    if source_project.layout != RepositoryLayout::Single
        || destination_project.layout != RepositoryLayout::Single
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
    let source_target = &source_project.environments[&source_environment].targets[from_target];
    let destination_target =
        &destination_project.environments[&destination_environment].targets[to_target];
    if source_target.application != destination_target.application {
        bail!("Promotion Targets use incompatible Applications");
    }
    let source_app = load_installed(&source_root, &source_target.application)?;
    let destination_app = load_installed(&destination_root, &destination_target.application)?;
    let source_baseline = load_baseline(&baseline_path(
        &source_root,
        &source_environment,
        from_target,
    ))
    .ok();
    let destination_baseline = load_baseline(&baseline_path(
        &destination_root,
        &destination_environment,
        to_target,
    ))
    .ok();
    let source_resolved = for_local_use(&source_app, source_target, source_baseline.as_ref())?;
    let destination_resolved = for_local_use(
        &destination_app,
        destination_target,
        destination_baseline.as_ref(),
    )?;
    if source_resolved.application_version != destination_resolved.application_version
        || source_resolved.catalog_version != destination_resolved.catalog_version
        || source_resolved.selected != destination_resolved.selected
    {
        bail!("Promotion installed Applications are not exactly compatible");
    }
    let selection = Selection {
        environment: None,
        targets: vec![from_target.into()],
        namespaces: vec![],
        types: vec![],
        ids: vec![],
    };
    let mut out = Vec::new();
    for item in list_inventory(&source_root, &selection)? {
        if item.id_scope != IdScope::Universal {
            bail!(
                "Resource {} has Target-scoped identity and cannot be promoted",
                item.id
            );
        }
        let filename = Path::new(&item.path).file_name().unwrap();
        let path = resource_directory_in_namespace(
            &destination_root,
            &destination_project,
            &destination_environment,
            to_target,
            item.namespace.as_deref(),
            &item.resource_type,
        )
        .join(filename);
        let resource_type = &destination_resolved.resource_types[&item.resource_type];
        let value = normalize_for_destination(
            DestinationScope {
                root: &destination_root,
                project: &destination_project,
                environment: &destination_environment,
                target: to_target,
                namespace: item.namespace.as_deref(),
                type_name: &item.resource_type,
            },
            resource_type,
            &item.value,
        )?;
        write_canonical_resource(&path, &value, resource_type)?;
        out.push(PromotionResult {
            from_environment: source_environment.clone(),
            to_environment: destination_environment.clone(),
            from_target: from_target.into(),
            to_target: to_target.into(),
            namespace: item.namespace,
            resource_type: Some(item.resource_type),
            id: item.id,
            outcome: "promoted".into(),
        });
    }
    Ok(out)
}
