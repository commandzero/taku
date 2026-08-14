use crate::application::load_installed;
use crate::canonical::{Selection, canonical_bytes, list_inventory, pointer_string};
use crate::project::current_environment;
use crate::provider::resolve_auth;
use crate::transport::{
    INTERNAL_CURSOR_POINTER, INTERNAL_GUARD_POINTER, OperationInput, RemoteResult,
    execute_retry_safe, remove_pointer,
};
use crate::variants::{ResolvedVariants, baseline_from, baseline_path, discover, save_baseline};
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFile {
    pub schema_version: u32,
    pub observed_at: DateTime<Utc>,
    pub binding: String,
    #[serde(default)]
    pub facts: BTreeMap<String, String>,
    #[serde(default)]
    pub variants: BTreeMap<String, String>,
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
}

pub fn fetch(
    root: &Path,
    selection: &Selection,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<FetchResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let inventory = list_inventory(&root, selection)?;
    let mut reports = Vec::new();
    let mut resolved: BTreeMap<String, ResolvedVariants> = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    let mut observation_files: BTreeMap<PathBuf, ObservationFile> = BTreeMap::new();
    for item in inventory {
        let target = &project.environments[&environment].targets[&item.target];
        let app = load_installed(&root, &target.application)?;
        if !resolved.contains_key(&item.target) {
            let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
            let discovered = discover(&app, target, &auth)?;
            let baseline = baseline_path(&root, &environment, &item.target);
            if !baseline.exists() {
                save_baseline(
                    &baseline,
                    &baseline_from(discovered.facts.clone(), discovered.selected.clone()),
                )?;
            }
            resolved.insert(item.target.clone(), discovered);
        }
        let discovered = &resolved[&item.target];
        let resource_type = &discovered.resource_types[&item.resource_type];
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
        if !bindings.contains_key(&item.target) {
            bindings.insert(
                item.target.clone(),
                binding(&root, &project, &environment, &item.target, &app)?,
            );
        }
        if !observation_files.contains_key(&path) {
            let file = if path.exists() {
                load_observation(&path)?
            } else {
                ObservationFile {
                    schema_version: SCHEMA_VERSION,
                    observed_at: Utc::now(),
                    binding: bindings[&item.target].clone(),
                    facts: BTreeMap::new(),
                    variants: BTreeMap::new(),
                    resources: BTreeMap::new(),
                }
            };
            observation_files.insert(path.clone(), file);
        }
        let file = observation_files.get_mut(&path).unwrap();
        file.observed_at = Utc::now();
        file.binding = bindings[&item.target].clone();
        file.facts = discovered.facts.clone();
        file.variants = discovered.selected.clone();
        let local_hash = hash(&canonical_bytes(&item.value)?);
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
                local_value: Some(item.value.clone()),
                path: item.path.clone(),
                present,
                value,
                guard,
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
                save_baseline(
                    &baseline,
                    &baseline_from(discovered.facts.clone(), discovered.selected.clone()),
                )?;
            }
            resolved.insert(marker.target.clone(), discovered);
        }
        let discovered = &resolved[&marker.target];
        let resource_type = &discovered.resource_types[&marker.resource_type];
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
        if !bindings.contains_key(&marker.target) {
            bindings.insert(
                marker.target.clone(),
                binding(&root, &project, &environment, &marker.target, &app)?,
            );
        }
        if !observation_files.contains_key(&path) {
            let file = if path.exists() {
                load_observation(&path)?
            } else {
                ObservationFile {
                    schema_version: SCHEMA_VERSION,
                    observed_at: Utc::now(),
                    binding: bindings[&marker.target].clone(),
                    facts: BTreeMap::new(),
                    variants: BTreeMap::new(),
                    resources: BTreeMap::new(),
                }
            };
            observation_files.insert(path.clone(), file);
        }
        let file = observation_files.get_mut(&path).unwrap();
        file.observed_at = Utc::now();
        file.binding = bindings[&marker.target].clone();
        file.facts = discovered.facts.clone();
        file.variants = discovered.selected.clone();
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
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let local = list_inventory(&root, selection)?;
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
    let env = &project.environments[&environment];
    let mut out = Vec::new();
    for (target_name, target) in &env.targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let discovered = discover(&app, target, &auth)?;
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
                    continue;
                }
                vec![None]
            };
            for namespace in namespaces {
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
                            if let Some(path) = &mut op.default_namespace_path {
                                append_query_parameter(path, page_parameter, &page.to_string());
                                append_query_parameter(path, size_parameter, &size.to_string());
                            }
                            match execute_retry_safe(
                                target,
                                &app,
                                rt,
                                &op,
                                OperationInput {
                                    namespace,
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
                                if let Some(path) = &mut op.default_namespace_path {
                                    append_query_parameter(path, cursor_parameter, &encoded);
                                }
                            }
                            let mut values = match execute_retry_safe(
                                target,
                                &app,
                                rt,
                                &op,
                                OperationInput {
                                    namespace,
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
                    let id = pointer_string(&value, &rt.id.pointer)
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
                    let name = pointer_string(&value, &rt.display_name.pointer)
                        .unwrap_or_else(|| id.clone());
                    out.push(RemoteEntry {
                        environment: environment.clone(),
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
            .join(format!("{resource_type}.yml")),
        None => target_root.join(format!("{resource_type}.yml")),
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
) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(serde_yaml::to_string(project)?);
    digest.update(environment);
    digest.update(target);
    digest.update(serde_yaml::to_string(app)?);
    let _ = root;
    Ok(hex::encode(digest.finalize()))
}
pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
