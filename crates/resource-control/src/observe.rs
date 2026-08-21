use crate::application::load_installed;
use crate::canonical::{Selection, canonical_bytes, list_inventory_with_resolved, pointer_string};
use crate::project::current_environment;
use crate::provider::resolve_auth;
use crate::resolution::{
    ResolvedApplication, baseline_from, baseline_path, discover, save_baseline,
};
use crate::transport::{
    INTERNAL_CURSOR_POINTER, INTERNAL_GUARD_POINTER, OperationInput, RemoteResult,
    execute_retry_safe, remove_pointer,
};
use crate::{SCHEMA_VERSION, git_root, load_project};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct FetchResult {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RemoteEntry {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub name: String,
    pub tracked: bool,
    #[serde(skip)]
    pub value: Value,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RemoteResourceType {
    pub name: String,
    pub namespaced: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFile {
    pub schema_version: u32,
    pub observed_at: DateTime<Utc>,
    pub binding: String,
    #[serde(default)]
    pub application_version: String,
    pub catalog_version: String,
    #[serde(default)]
    pub definitions: BTreeMap<String, String>,
    pub resources: BTreeMap<String, Observation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub local_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_value: Option<Value>,
    pub path: String,
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub requires_pull: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint_binding: Option<String>,
}

fn requires_pull_after_configuration_change(
    previous: Option<&Observation>,
    hints: &crate::hints::HintResolution,
    current_binding: &str,
    structural_binding_changed: bool,
) -> bool {
    previous.is_some_and(|observation| {
        observation.requires_pull
            || structural_binding_changed
            || observation.hint_binding.as_ref().map_or_else(
                || hints.target_bytes.is_some() || hints.resource_bytes.is_some(),
                |binding| binding != current_binding,
            )
    })
}

pub fn fetch(
    root: &Path,
    selection: &Selection,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<FetchResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    crate::hints::validate_project_placement(&root, &project)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut resolved: BTreeMap<String, ResolvedApplication> = BTreeMap::new();
    let mut missing_baselines = Vec::new();
    for (target_name, target) in &project.environments[&environment].targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        crate::hints::validate_application_tree(&root, &project, &environment, target_name, &app)?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let discovered = discover(&app, target, &auth)?;
        let baseline = baseline_path(&root, &environment, target_name);
        if !baseline.exists() {
            missing_baselines.push((baseline, baseline_from(&discovered)));
        }
        resolved.insert(target_name.clone(), discovered);
    }
    let inventory = list_inventory_with_resolved(&root, selection, Some(&resolved))?;
    for (path, baseline) in missing_baselines {
        save_baseline(&path, &baseline)?;
    }
    let mut reports = Vec::new();
    let mut bindings = BTreeMap::new();
    let mut binding_changes = BTreeMap::new();
    let mut observation_files: BTreeMap<PathBuf, ObservationFile> = BTreeMap::new();
    for item in inventory {
        let target = &project.environments[&environment].targets[&item.target];
        let app = load_installed(&root, &target.application)?;
        if !resolved.contains_key(&item.target) {
            let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
            let discovered = discover(&app, target, &auth)?;
            let baseline = baseline_path(&root, &environment, &item.target);
            if !baseline.exists() {
                save_baseline(&baseline, &baseline_from(&discovered))?;
            }
            resolved.insert(item.target.clone(), discovered);
        }
        let discovered = &resolved[&item.target];
        let resource_type = &discovered.resource_types[&item.resource_type];
        let hint_resolution = crate::hints::resolve(
            &root,
            &project,
            &environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        )?;
        let metadata_track = hint_resolution.track;
        let hint_binding = hash(&hint_resolution.binding_material(&root)?);
        let operation = resource_type
            .operations
            .read
            .as_ref()
            .context("selected Resource Type has no Read Operation")?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let remote = if item.pending {
            RemoteResult::NotFound
        } else {
            match execute_retry_safe(
                target,
                &app,
                resource_type,
                operation,
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
            ) {
                Ok(remote) => remote,
                Err(error) if is_transformation_conflict(&error) => {
                    reports.push(FetchResult {
                        environment: environment.clone(),
                        target: item.target,
                        namespace: item.namespace,
                        resource_type: item.resource_type,
                        id: item.id,
                        outcome: "transformation_conflict".into(),
                    });
                    continue;
                }
                Err(error) => return Err(error),
            }
        };
        let (present, mut value, outcome) = match remote {
            RemoteResult::Success(mut values) if values.len() == 1 => {
                (true, values.pop(), "observed")
            }
            RemoteResult::NotFound => (false, None, "absent"),
            RemoteResult::Conflict => bail!("Read Operation reported a conflict for {}", item.id),
            RemoteResult::Retryable | RemoteResult::Uncertain => {
                bail!("Read Operation failed transiently for {}", item.id)
            }
            RemoteResult::Failure(message) => {
                bail!("Read Operation failed for {}: {message}", item.id)
            }
            RemoteResult::Success(_) => bail!("One Read Operation returned multiple Resources"),
        };
        let path = cache_path(
            &root,
            &environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        );
        if !bindings.contains_key(&path) {
            bindings.insert(
                path.clone(),
                binding(
                    &root,
                    &project,
                    &environment,
                    &item.target,
                    &app,
                    item.namespace.as_deref(),
                    &item.resource_type,
                )?,
            );
        }
        if !observation_files.contains_key(&path) {
            let file = if path.exists() {
                load_observation(&path)?
            } else {
                ObservationFile {
                    schema_version: SCHEMA_VERSION,
                    observed_at: Utc::now(),
                    binding: bindings[&path].clone(),
                    application_version: discovered.application_version.clone(),
                    catalog_version: discovered.catalog_version.clone(),
                    definitions: BTreeMap::new(),
                    resources: BTreeMap::new(),
                }
            };
            binding_changes.insert(path.clone(), file.binding != bindings[&path]);
            observation_files.insert(path.clone(), file);
        }
        let file = observation_files.get_mut(&path).unwrap();
        let requires_pull = requires_pull_after_configuration_change(
            file.resources.get(&item.id),
            &hint_resolution,
            &hint_binding,
            binding_changes[&path],
        );
        let previous_local = requires_pull.then(|| {
            file.resources
                .get(&item.id)
                .map(|resource| (resource.local_hash.clone(), resource.local_value.clone()))
        });
        file.observed_at = Utc::now();
        file.binding = bindings[&path].clone();
        file.application_version = discovered.application_version.clone();
        file.catalog_version = discovered.catalog_version.clone();
        file.definitions = discovered.selected.clone();
        let current_local_hash = hash(&canonical_bytes(&item.value)?);
        let (local_hash, local_value) = previous_local
            .flatten()
            .unwrap_or_else(|| (current_local_hash, Some(item.value.clone())));
        let guard = value.as_ref().map(|value| {
            pointer_string(value, INTERNAL_GUARD_POINTER)
                .unwrap_or_else(|| hash(&canonical_bytes(value).unwrap_or_default()))
        });
        if let Some(value) = &mut value {
            remove_pointer(value, INTERNAL_GUARD_POINTER)?;
        }
        file.resources.insert(
            item.id.clone(),
            Observation {
                local_hash,
                local_value,
                path: item.path.clone(),
                present,
                value,
                guard,
                requires_pull,
                hint_binding: Some(hint_binding),
            },
        );
        reports.push(FetchResult {
            environment: environment.clone(),
            target: item.target,
            namespace: item.namespace,
            resource_type: item.resource_type,
            id: item.id,
            outcome: outcome.into(),
        });
    }
    for (marker_path, marker) in crate::lifecycle::deletion_markers(&root, selection)? {
        let target = &project.environments[&environment].targets[&marker.target];
        let app = load_installed(&root, &target.application)?;
        if !resolved.contains_key(&marker.target) {
            let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
            let discovered = discover(&app, target, &auth)?;
            let baseline = baseline_path(&root, &environment, &marker.target);
            if !baseline.exists() {
                save_baseline(&baseline, &baseline_from(&discovered))?;
            }
            resolved.insert(marker.target.clone(), discovered);
        }
        let discovered = &resolved[&marker.target];
        let resource_type = &discovered.resource_types[&marker.resource_type];
        let hint_resolution = crate::hints::resolve(
            &root,
            &project,
            &environment,
            &marker.target,
            marker.namespace.as_deref(),
            &marker.resource_type,
        )?;
        let metadata_track = hint_resolution.track;
        let hint_binding = hash(&hint_resolution.binding_material(&root)?);
        let operation = resource_type
            .operations
            .read
            .as_ref()
            .context("Deletion Marker Resource Type has no Read Operation")?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let marker_context = Value::Object(
            marker
                .parameters
                .iter()
                .map(|(name, value)| (name.clone(), Value::String(value.clone())))
                .collect(),
        );
        let remote = match execute_retry_safe(
            target,
            &app,
            resource_type,
            operation,
            OperationInput {
                namespace: marker.namespace.as_deref(),
                id: Some(&marker.id),
                context: Some(&marker_context),
                body: None,
                resource_ids: None,
                mutation: false,
                metadata_track,
            },
            &auth,
        ) {
            Ok(remote) => remote,
            Err(error) if is_transformation_conflict(&error) => {
                reports.push(FetchResult {
                    environment: environment.clone(),
                    target: marker.target,
                    namespace: marker.namespace,
                    resource_type: marker.resource_type,
                    id: marker.id,
                    outcome: "transformation_conflict".into(),
                });
                continue;
            }
            Err(error) => return Err(error),
        };
        let (present, mut value, outcome) = match remote {
            RemoteResult::Success(mut values) if values.len() == 1 => {
                (true, values.pop(), "observed")
            }
            RemoteResult::NotFound => (false, None, "absent"),
            RemoteResult::Conflict => {
                bail!("Read Operation reported a conflict for {}", marker.id)
            }
            RemoteResult::Retryable | RemoteResult::Uncertain => {
                bail!("Read Operation failed transiently for {}", marker.id)
            }
            RemoteResult::Failure(message) => {
                bail!("Read Operation failed for {}: {message}", marker.id)
            }
            RemoteResult::Success(_) => bail!("One Read Operation returned multiple Resources"),
        };
        let path = cache_path(
            &root,
            &environment,
            &marker.target,
            marker.namespace.as_deref(),
            &marker.resource_type,
        );
        if !bindings.contains_key(&path) {
            bindings.insert(
                path.clone(),
                binding(
                    &root,
                    &project,
                    &environment,
                    &marker.target,
                    &app,
                    marker.namespace.as_deref(),
                    &marker.resource_type,
                )?,
            );
        }
        if !observation_files.contains_key(&path) {
            let file = if path.exists() {
                load_observation(&path)?
            } else {
                ObservationFile {
                    schema_version: SCHEMA_VERSION,
                    observed_at: Utc::now(),
                    binding: bindings[&path].clone(),
                    application_version: discovered.application_version.clone(),
                    catalog_version: discovered.catalog_version.clone(),
                    definitions: BTreeMap::new(),
                    resources: BTreeMap::new(),
                }
            };
            binding_changes.insert(path.clone(), file.binding != bindings[&path]);
            observation_files.insert(path.clone(), file);
        }
        let file = observation_files.get_mut(&path).unwrap();
        let requires_pull = requires_pull_after_configuration_change(
            file.resources.get(&marker.id),
            &hint_resolution,
            &hint_binding,
            binding_changes[&path],
        );
        file.observed_at = Utc::now();
        file.binding = bindings[&path].clone();
        file.application_version = discovered.application_version.clone();
        file.catalog_version = discovered.catalog_version.clone();
        file.definitions = discovered.selected.clone();
        let guard = value.as_ref().map(|value| {
            pointer_string(value, INTERNAL_GUARD_POINTER)
                .unwrap_or_else(|| hash(&canonical_bytes(value).unwrap_or_default()))
        });
        if let Some(value) = &mut value {
            remove_pointer(value, INTERNAL_GUARD_POINTER)?;
        }
        file.resources.insert(
            marker.id.clone(),
            Observation {
                local_hash: hash(marker.guard.as_bytes()),
                local_value: None,
                path: marker_path
                    .strip_prefix(&root)
                    .unwrap()
                    .display()
                    .to_string(),
                present,
                value,
                guard,
                requires_pull,
                hint_binding: Some(hint_binding),
            },
        );
        reports.push(FetchResult {
            environment: environment.clone(),
            target: marker.target,
            namespace: marker.namespace,
            resource_type: marker.resource_type,
            id: marker.id,
            outcome: outcome.into(),
        });
    }
    for (path, file) in observation_files {
        save_observation(&path, &file)?;
    }
    reports.sort_by(|a, b| {
        (&a.environment, &a.target, &a.resource_type, &a.id).cmp(&(
            &b.environment,
            &b.target,
            &b.resource_type,
            &b.id,
        ))
    });
    Ok(reports)
}

pub fn is_transformation_conflict(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains("Transformation Conflict"))
}

pub fn remote_list(
    root: &Path,
    selection: &Selection,
    untracked_only: bool,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteEntry>> {
    let discovery = discover_remote(root, selection, cli_provider)?;
    for (path, baseline) in &discovery.missing_baselines {
        save_baseline(path, baseline)?;
    }
    remote_query(&discovery, selection, untracked_only, cli_provider)
}

/// Query remote Resources without persisting Baselines or any other Project state.
pub fn remote_list_read_only(
    root: &Path,
    selection: &Selection,
    untracked_only: bool,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteEntry>> {
    let discovery = discover_remote(root, selection, cli_provider)?;
    remote_query(&discovery, selection, untracked_only, cli_provider)
}

/// Discover the effective listable Resource Types for one Target without persistence.
pub fn remote_resource_types_read_only(
    root: &Path,
    environment: Option<&str>,
    target_name: &str,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteResourceType>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    crate::hints::validate_project_placement(&root, &project)?;
    let environment = current_environment(&root, &project, environment)?;
    let target = project.environments[&environment]
        .targets
        .get(target_name)
        .with_context(|| format!("unknown Target {target_name}"))?;
    let app = load_installed(&root, &target.application)?;
    crate::hints::validate_application_tree(&root, &project, &environment, target_name, &app)?;
    let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
    Ok(discover(&app, target, &auth)?
        .resource_types
        .into_iter()
        .filter(|(_, resource_type)| {
            resource_type
                .operations
                .list
                .as_ref()
                .is_some_and(|operation| operation.cardinality == crate::Cardinality::Many)
        })
        .map(|(name, resource_type)| RemoteResourceType {
            name,
            namespaced: resource_type.namespaced,
        })
        .collect())
}

struct RemoteDiscovery {
    root: PathBuf,
    project: crate::Project,
    environment: String,
    resolved: BTreeMap<String, ResolvedApplication>,
    missing_baselines: Vec<(PathBuf, crate::TargetBaseline)>,
}

fn discover_remote(
    root: &Path,
    selection: &Selection,
    cli_provider: &BTreeMap<String, String>,
) -> Result<RemoteDiscovery> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    crate::hints::validate_project_placement(&root, &project)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut resolved = BTreeMap::new();
    let mut missing_baselines = Vec::new();
    for (target_name, target) in &project.environments[&environment].targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        crate::hints::validate_application_tree(&root, &project, &environment, target_name, &app)?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let discovered = discover(&app, target, &auth)?;
        let baseline = baseline_path(&root, &environment, target_name);
        if !baseline.exists() {
            missing_baselines.push((baseline, baseline_from(&discovered)));
        }
        resolved.insert(target_name.clone(), discovered);
    }
    Ok(RemoteDiscovery {
        root,
        project,
        environment,
        resolved,
        missing_baselines,
    })
}

fn remote_query(
    discovery: &RemoteDiscovery,
    selection: &Selection,
    untracked_only: bool,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteEntry>> {
    let root = &discovery.root;
    let project = &discovery.project;
    let environment = discovery.environment.as_str();
    let resolved = &discovery.resolved;
    let local = list_inventory_with_resolved(root, selection, Some(resolved))?;
    let local_ids: std::collections::BTreeSet<_> = local
        .iter()
        .map(|i| {
            (
                i.target.clone(),
                i.namespace.clone(),
                i.resource_type.clone(),
                i.id.clone(),
            )
        })
        .collect();
    let env = &project.environments[environment];
    let mut out = Vec::new();
    for (target_name, target) in &env.targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(root, &target.application)?;
        let auth = resolve_auth(root, project, environment, target, cli_provider)?;
        let discovered = &resolved[target_name];
        for (type_name, rt) in &discovered.resource_types {
            if !selection.types.is_empty() && !selection.types.contains(type_name) {
                continue;
            }
            let operation = rt
                .operations
                .list
                .as_ref()
                .context("selected Resource Type has no Many List Operation")?;
            if operation.cardinality != crate::Cardinality::Many {
                bail!("List Operation must have Many cardinality");
            }
            let namespaces: Vec<Option<&str>> = if rt.namespaced {
                if selection.namespaces.is_empty() {
                    bail!(
                        "--namespace is required to list remote Resources of namespaced type {type_name}"
                    );
                }
                selection
                    .namespaces
                    .iter()
                    .map(|value| Some(value.as_str()))
                    .collect()
            } else {
                if !selection.namespaces.is_empty() {
                    bail!("--namespace is not valid for non-namespaced Resource Type {type_name}");
                }
                vec![None]
            };
            for namespace in namespaces {
                let metadata_track = crate::hints::resolve(
                    root,
                    project,
                    environment,
                    target_name,
                    namespace,
                    type_name,
                )?
                .track;
                let mut resources = Vec::new();
                match &operation.pagination {
                    Some(crate::Pagination::PageSize {
                        page_parameter,
                        size_parameter,
                        size,
                        max_pages,
                    }) => {
                        for page in 1..=*max_pages {
                            let mut op = operation.clone();
                            append_query_parameter(&mut op.path, page_parameter, &page.to_string());
                            append_query_parameter(&mut op.path, size_parameter, &size.to_string());
                            match execute_retry_safe(
                                target,
                                &app,
                                rt,
                                &op,
                                OperationInput {
                                    namespace,
                                    metadata_track,
                                    ..OperationInput::default()
                                },
                                &auth,
                            )? {
                                RemoteResult::Success(values) => {
                                    let count = values.len();
                                    resources.extend(values);
                                    if count < *size {
                                        break;
                                    }
                                    if page == *max_pages {
                                        bail!("pagination exceeded configured max_pages");
                                    }
                                }
                                RemoteResult::NotFound => break,
                                RemoteResult::Conflict => bail!("List Operation reported conflict"),
                                RemoteResult::Retryable | RemoteResult::Uncertain => {
                                    bail!("List Operation failed transiently")
                                }
                                RemoteResult::Failure(message) => {
                                    bail!("List Operation failed: {message}")
                                }
                            }
                        }
                    }
                    Some(crate::Pagination::Cursor {
                        cursor_parameter,
                        next_pointer: _,
                        max_pages,
                    }) => {
                        let mut cursor: Option<String> = None;
                        let mut seen_cursors = std::collections::BTreeSet::new();
                        for page in 0..*max_pages {
                            let mut op = operation.clone();
                            if let Some(value) = &cursor {
                                let encoded = urlencoding::encode(value);
                                append_query_parameter(&mut op.path, cursor_parameter, &encoded);
                            }
                            let mut values = match execute_retry_safe(
                                target,
                                &app,
                                rt,
                                &op,
                                OperationInput {
                                    namespace,
                                    metadata_track,
                                    ..OperationInput::default()
                                },
                                &auth,
                            )? {
                                RemoteResult::Success(values) => values,
                                RemoteResult::NotFound => break,
                                RemoteResult::Conflict => bail!("List Operation reported conflict"),
                                RemoteResult::Retryable | RemoteResult::Uncertain => {
                                    bail!("List Operation failed transiently")
                                }
                                RemoteResult::Failure(message) => {
                                    bail!("List Operation failed: {message}")
                                }
                            };
                            let next = values
                                .last()
                                .and_then(|value| pointer_string(value, INTERNAL_CURSOR_POINTER));
                            for value in &mut values {
                                remove_pointer(value, INTERNAL_CURSOR_POINTER)?;
                            }
                            resources.extend(values);
                            if next.is_none() {
                                break;
                            }
                            if !seen_cursors.insert(next.clone().unwrap()) {
                                bail!("cursor pagination repeated a continuation cursor");
                            }
                            cursor = next;
                            if page + 1 == *max_pages {
                                bail!("pagination exceeded configured max_pages");
                            }
                        }
                    }
                    None => match execute_retry_safe(
                        target,
                        &app,
                        rt,
                        operation,
                        OperationInput {
                            namespace,
                            metadata_track,
                            ..OperationInput::default()
                        },
                        &auth,
                    )? {
                        RemoteResult::Success(values) => resources = values,
                        RemoteResult::NotFound => {}
                        RemoteResult::Conflict => bail!("List Operation reported conflict"),
                        RemoteResult::Retryable | RemoteResult::Uncertain => {
                            bail!("List Operation failed transiently")
                        }
                        RemoteResult::Failure(message) => bail!("List Operation failed: {message}"),
                    },
                };
                let mut seen_ids = std::collections::BTreeSet::new();
                for mut value in resources {
                    remove_pointer(&mut value, INTERNAL_GUARD_POINTER)?;
                    let id = crate::canonical::canonical_id(&value, rt)
                        .context("remote Resource has no configured identity")?;
                    if !selection.ids.is_empty() && !selection.ids.contains(&id) {
                        continue;
                    }
                    if !seen_ids.insert(id.clone()) {
                        bail!("selected List Operation returned duplicate Resource ID {id}");
                    }
                    let tracked = local_ids.contains(&(
                        target_name.clone(),
                        namespace.map(str::to_owned),
                        type_name.clone(),
                        id.clone(),
                    ));
                    if untracked_only && tracked {
                        continue;
                    }
                    let name = crate::canonical::display_name_value(&value, &rt.display_name)
                        .unwrap_or_else(|| id.clone());
                    out.push(RemoteEntry {
                        environment: environment.to_owned(),
                        target: target_name.clone(),
                        namespace: namespace.map(str::to_owned),
                        resource_type: type_name.clone(),
                        id,
                        name,
                        tracked,
                        value,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        (&a.target, &a.namespace, &a.resource_type, &a.id).cmp(&(
            &b.target,
            &b.namespace,
            &b.resource_type,
            &b.id,
        ))
    });
    Ok(out)
}

fn append_query_parameter(path: &mut String, name: &str, value: &str) {
    let separator = if path.contains('?') { '&' } else { '?' };
    path.push(separator);
    path.push_str(name);
    path.push('=');
    path.push_str(value);
}

pub fn cache_path(
    root: &Path,
    environment: &str,
    target: &str,
    namespace: Option<&str>,
    resource_type: &str,
) -> PathBuf {
    let target_root = root.join(".taku/cache").join(environment).join(target);
    match namespace {
        Some(namespace) => target_root
            .join(namespace)
            .join(format!("{resource_type}.yaml")),
        None => target_root.join(format!("{resource_type}.yaml")),
    }
}
pub fn load_observation(path: &Path) -> Result<ObservationFile> {
    let file: ObservationFile = serde_yaml::from_str(
        &fs::read_to_string(path)
            .context("Observed State Cache is unavailable; run `taku fetch`")?,
    )
    .context("invalid Observed State Cache")?;
    if file.schema_version != SCHEMA_VERSION {
        bail!("unsupported Observed State schema version");
    }
    Ok(file)
}
pub fn save_observation(path: &Path, file: &ObservationFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_yaml::to_string(file)?)?;
    Ok(())
}
pub fn binding(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    target: &str,
    app: &crate::ApplicationDefinition,
    namespace: Option<&str>,
    resource_type: &str,
) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(serde_yaml::to_string(project)?);
    digest.update(environment);
    digest.update(target);
    digest.update(serde_yaml::to_string(app)?);
    for (major, catalog) in &app.catalogs {
        digest.update(major.to_be_bytes());
        digest.update(serde_yaml::to_string(catalog)?);
    }
    let hints =
        crate::hints::resolve(root, project, environment, target, namespace, resource_type)?;
    digest.update(hints.binding_material(root)?);
    Ok(hex::encode(digest.finalize()))
}
pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
