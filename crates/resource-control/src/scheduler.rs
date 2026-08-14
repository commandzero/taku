use crate::application::load_installed;
use crate::canonical::{
    InventoryEntry, Selection, canonical_bytes, list_inventory, owned_value, pointer_string,
    write_resource,
};
use crate::lifecycle::{DeletionMarker, deletion_markers};
use crate::observe::{binding, cache_path, hash, load_observation, save_observation};
use crate::project::current_environment;
use crate::provider::{SecretFields, resolve_auth};
use crate::reconcile::PushResult;
use crate::transport::{
    INTERNAL_GUARD_POINTER, OperationInput, RemoteResult, execute, execute_retry_safe, outbound,
    remove_pointer,
};
use crate::variants::{baseline_path, discover, from_baseline, load_baseline};
use crate::{
    ApplicationDefinition, ConcurrencyClass, GitPolicy, MissingPolicy, Operation, ResourceType,
    TargetConfig, WriteIntent, git_root, load_project,
};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

type GroupKey = (String, Option<String>, String);

#[derive(Clone)]
struct PreparedResource {
    item: InventoryEntry,
    target: TargetConfig,
    app: ApplicationDefinition,
    effective_resource_types: BTreeMap<String, ResourceType>,
    resource_type: ResourceType,
    operation: Operation,
    auth: SecretFields,
    observation_path: PathBuf,
}
impl PreparedResource {
    fn group(&self) -> GroupKey {
        (
            self.item.target.clone(),
            self.item.namespace.clone(),
            self.item.resource_type.clone(),
        )
    }
    fn key(&self) -> String {
        format!(
            "{}/{}/{}/{}:write",
            self.item.target,
            self.item.namespace.as_deref().unwrap_or("-"),
            self.item.resource_type,
            self.item.id
        )
    }
}
#[derive(Clone)]
struct PreparedDeletion {
    path: PathBuf,
    marker: DeletionMarker,
    target: TargetConfig,
    app: ApplicationDefinition,
    effective_resource_types: BTreeMap<String, ResourceType>,
    resource_type: ResourceType,
    read: Operation,
    delete: Operation,
    auth: SecretFields,
    remote_absent: bool,
}
impl PreparedDeletion {
    fn key(&self) -> String {
        format!(
            "{}/{}/{}/{}:delete",
            self.marker.target,
            self.marker.namespace.as_deref().unwrap_or("-"),
            self.marker.resource_type,
            self.marker.id
        )
    }
}

struct CurrentRemote {
    present: bool,
    value: Option<serde_json::Value>,
    guard: Option<String>,
}

enum DeletionVerification {
    Present,
    Absent,
    Conflict,
}

enum ExecutionJob {
    One(GroupKey, Box<PreparedResource>),
    Many(GroupKey, Vec<PreparedResource>),
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PushJournal {
    schema_version: u32,
    binding: String,
    total: usize,
    completed: BTreeMap<String, String>,
}

#[allow(clippy::too_many_arguments)]
pub fn push(
    root: &Path,
    selection: &Selection,
    dry_run: bool,
    uncommitted: Option<GitPolicy>,
    untracked: Option<GitPolicy>,
    interactive: bool,
    confirmed: bool,
    new_plan: bool,
    missing: Option<MissingPolicy>,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<PushResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let inventory = list_inventory(&root, selection)?;
    let mut preflight_conflicts =
        preflight_transformations(&root, &project, &environment, &inventory)?;
    if !preflight_conflicts.is_empty() {
        let revision = git_revision(&root)?;
        sort_reports(&mut preflight_conflicts);
        stamp_reports(&mut preflight_conflicts, &revision);
        return Ok(preflight_conflicts);
    }
    let markers = deletion_markers(&root, selection)?;
    let marker_paths: Vec<_> = markers
        .iter()
        .map(|(path, _)| path.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    enforce_git(
        &root,
        &inventory,
        &marker_paths,
        uncommitted.or(project.push.uncommitted),
        untracked.or(project.push.untracked),
        interactive,
        confirmed,
    )?;
    let revision = git_revision(&root)?;
    let mut reports = Vec::new();
    let mut prepared = Vec::new();
    let mut preparation_failed = BTreeSet::new();
    let mut observation_files = BTreeMap::new();
    let mut binding_parts = vec![serde_yaml::to_string(&project)?, revision.clone()];
    for item in inventory {
        let target = &project.environments[&environment].targets[&item.target];
        let app = load_installed(&root, &target.application)?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let discovered = match discover(&app, target, &auth) {
            Ok(discovered) => discovered,
            Err(_) => {
                reports.push(report(&environment, &item, "failed", "target_discovery"));
                preparation_failed.insert(inventory_group(&item));
                continue;
            }
        };
        let baseline = load_baseline(&baseline_path(&root, &environment, &item.target))
            .context("Target Baseline is unavailable; run `taku fetch`")?;
        if baseline.variants != discovered.selected {
            bail!(
                "Target Facts select different Resource Type Variants; run `taku fetch` and `taku pull`"
            );
        }
        let resource_type = discovered.resource_types[&item.resource_type].clone();
        binding_parts.push(format!(
            "{}:{}:{}:{}",
            item.target,
            item.resource_type,
            item.id,
            hash(&canonical_bytes(&item.value)?)
        ));
        binding_parts.push(serde_yaml::to_string(&app)?);
        binding_parts.push(serde_yaml::to_string(&baseline)?);
        let observation_path = cache_path(
            &root,
            &environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        );
        if !observation_files.contains_key(&observation_path) {
            observation_files.insert(
                observation_path.clone(),
                load_observation(&observation_path)?,
            );
        }
        let observation_file = &observation_files[&observation_path];
        if observation_file.binding != binding(&root, &project, &environment, &item.target, &app)? {
            bail!("Observed State is structurally invalid; run `taku fetch`");
        }
        let observation = observation_file
            .resources
            .get(&item.id)
            .context("selected Resource is absent from Observed State")?;
        let current = if resource_type.write_intent != WriteIntent::Upsert
            || resource_type.concurrency_mode == crate::ConcurrencyMode::Guarded
        {
            match read_current(
                target,
                &app,
                &resource_type,
                &auth,
                item.namespace.as_deref(),
                &item.id,
                Some(&item.value),
            ) {
                Ok(current) => current,
                Err(_) => {
                    reports.push(report(&environment, &item, "failed", "targeted_read"));
                    preparation_failed.insert(inventory_group(&item));
                    continue;
                }
            }
        } else {
            CurrentRemote {
                present: observation.present,
                value: observation.value.clone(),
                guard: observation.guard.clone(),
            }
        };
        if current.present
            && resource_type.write_intent != WriteIntent::Create
            && current.value.as_ref().is_some_and(|value| {
                hash(
                    &canonical_bytes(&owned_value(
                        value,
                        &item.value,
                        resource_type.mutation_mode,
                    ))
                    .unwrap_or_default(),
                ) == hash(&canonical_bytes(&item.value).unwrap_or_default())
            })
        {
            reports.push(report(
                &environment,
                &item,
                "in_sync",
                &format!("{:?}", resource_type.concurrency_mode).to_ascii_lowercase(),
            ));
            continue;
        }
        if !current.present && resource_type.write_intent != WriteIntent::Create {
            let policy = missing
                .or(resource_type.missing.push)
                .unwrap_or(MissingPolicy::Conflict);
            match policy {
                MissingPolicy::Conflict => {
                    reports.push(report(
                        &environment,
                        &item,
                        "presence_conflict",
                        &format!("{:?}", resource_type.concurrency_mode).to_ascii_lowercase(),
                    ));
                    preparation_failed.insert(inventory_group(&item));
                    continue;
                }
                MissingPolicy::Delete => {
                    reports.push(report(&environment, &item, "failed", "missing_policy"));
                    preparation_failed.insert(inventory_group(&item));
                    continue;
                }
                MissingPolicy::Restore => {}
            }
        }
        let operation = match resource_type.write_intent {
            WriteIntent::Create => {
                if current.present {
                    reports.push(report(
                        &environment,
                        &item,
                        "creation_conflict",
                        "existence_guard",
                    ));
                    preparation_failed.insert(inventory_group(&item));
                    continue;
                }
                resource_type.operations.create.clone()
            }
            WriteIntent::Update => {
                if !current.present {
                    reports.push(report(
                        &environment,
                        &item,
                        "presence_conflict",
                        "existence_guard",
                    ));
                    preparation_failed.insert(inventory_group(&item));
                    continue;
                }
                resource_type.operations.update.clone()
            }
            WriteIntent::Upsert => resource_type.operations.upsert.clone().or_else(|| {
                if current.present {
                    resource_type.operations.update.clone()
                } else {
                    resource_type.operations.create.clone()
                }
            }),
        };
        let Some(mut operation) = operation else {
            reports.push(report(&environment, &item, "failed", "write_intent"));
            preparation_failed.insert(inventory_group(&item));
            continue;
        };
        if resource_type.concurrency_mode == crate::ConcurrencyMode::Guarded && current.present {
            let (Some(header), Some(guard)) = (operation.guard_header.clone(), current.guard)
            else {
                reports.push(report(&environment, &item, "failed", "concurrency_guard"));
                preparation_failed.insert(inventory_group(&item));
                continue;
            };
            operation.headers.insert(header, guard);
        }
        prepared.push(PreparedResource {
            item,
            target: target.clone(),
            app,
            effective_resource_types: discovered.resource_types,
            resource_type,
            operation,
            auth,
            observation_path,
        });
    }
    let mut prepared_deletions = Vec::new();
    for (path, marker) in markers {
        let target = &project.environments[&environment].targets[&marker.target];
        let app = load_installed(&root, &target.application)?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let discovered = match discover(&app, target, &auth) {
            Ok(discovered) => discovered,
            Err(_) => {
                preparation_failed.insert(marker_group(&marker));
                reports.push(deletion_report(&environment, &marker, "failed"));
                continue;
            }
        };
        let baseline = load_baseline(&baseline_path(&root, &environment, &marker.target))
            .context("Target Baseline is unavailable; run `taku fetch`")?;
        if baseline.variants != discovered.selected {
            bail!(
                "Target Facts select different Resource Type Variants; run `taku fetch` and `taku pull`"
            );
        }
        let resource_type = discovered.resource_types[&marker.resource_type].clone();
        let read = resource_type
            .operations
            .read
            .clone()
            .context("Deletion guard requires a Read Operation")?;
        let delete = resource_type
            .operations
            .delete
            .clone()
            .context("Resource Type has no Delete Operation")?;
        binding_parts.push(format!(
            "{}:{}:{}:{}",
            marker.target, marker.resource_type, marker.id, marker.guard
        ));
        let mut prepared = PreparedDeletion {
            path,
            marker,
            target: target.clone(),
            app,
            effective_resource_types: discovered.resource_types,
            resource_type,
            read,
            delete,
            auth,
            remote_absent: false,
        };
        match verify_deletion(&prepared) {
            Err(_) => {
                preparation_failed.insert(marker_group(&prepared.marker));
                reports.push(deletion_report(&environment, &prepared.marker, "failed"));
            }
            Ok(DeletionVerification::Present) => prepared_deletions.push(prepared),
            Ok(DeletionVerification::Absent) => {
                prepared.remote_absent = true;
                prepared_deletions.push(prepared);
            }
            Ok(DeletionVerification::Conflict) => {
                preparation_failed.insert(marker_group(&prepared.marker));
                reports.push(deletion_report(
                    &environment,
                    &prepared.marker,
                    "deletion_conflict",
                ));
            }
        }
    }
    if dry_run {
        for task in prepared {
            reports.push(report(
                &environment,
                &task.item,
                "planned",
                &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
            ));
        }
        for task in prepared_deletions {
            reports.push(PushResult {
                environment: environment.clone(),
                target: task.marker.target,
                namespace: task.marker.namespace,
                resource_type: task.marker.resource_type,
                id: task.marker.id,
                outcome: if task.remote_absent {
                    "planned_already_absent".into()
                } else {
                    "planned_delete".into()
                },
                git_revision: String::new(),
                safety: "guarded_deletion_marker".into(),
            });
        }
        sort_reports(&mut reports);
        stamp_reports(&mut reports, &revision);
        return Ok(reports);
    }
    binding_parts.sort();
    let plan_binding = hash(binding_parts.join("\0").as_bytes());
    let total = prepared.len() + prepared_deletions.len();
    let journal_path = root
        .join(".taku/journals")
        .join(&environment)
        .join("push.yml");
    let journal = prepare_journal(&journal_path, &plan_binding, total, new_plan)?;
    let journal = Arc::new(Mutex::new(journal));
    let serial = Arc::new(Mutex::new(()));
    let persistence = Arc::new(Mutex::new(()));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(project.max_requests)
        .build()?;
    let mut groups: BTreeMap<GroupKey, Vec<PreparedResource>> = BTreeMap::new();
    for task in prepared {
        groups.entry(task.group()).or_default().push(task);
    }
    let mut successful = BTreeSet::new();
    let mut failed = preparation_failed;
    while !groups.is_empty() {
        let blocked: Vec<_> = groups
            .keys()
            .filter(|key| {
                dependency_groups(&groups[key])
                    .iter()
                    .any(|dependency| failed.contains(dependency))
            })
            .cloned()
            .collect();
        for key in blocked {
            for task in groups.remove(&key).unwrap() {
                reports.push(report(
                    &environment,
                    &task.item,
                    "blocked_dependency",
                    &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
                ));
            }
            failed.insert(key);
        }
        let ready: Vec<_> = groups
            .keys()
            .filter(|key| {
                dependency_groups(&groups[key]).iter().all(|dependency| {
                    successful.contains(dependency) || !groups.contains_key(dependency)
                })
            })
            .cloned()
            .collect();
        if ready.is_empty() && !groups.is_empty() {
            bail!("Resource Type dependency cycle prevents Push execution");
        }
        let mut jobs = Vec::new();
        for key in &ready {
            let tasks = groups.remove(key).unwrap();
            if tasks
                .first()
                .is_some_and(|task| task.operation.cardinality == crate::Cardinality::Many)
            {
                jobs.push(ExecutionJob::Many(key.clone(), tasks));
            } else {
                for task in tasks {
                    jobs.push(ExecutionJob::One(key.clone(), Box::new(task)));
                }
            }
        }
        let results: Vec<(GroupKey, Vec<PushResult>)> = pool.install(|| {
            jobs.into_par_iter()
                .map(|job| match job {
                    ExecutionJob::One(key, task) => {
                        let fallback_item = task.item.clone();
                        let safety = format!("{:?}", task.resource_type.concurrency_mode)
                            .to_ascii_lowercase();
                        let result = execute_resource(
                            &root,
                            &environment,
                            *task,
                            &journal_path,
                            &journal,
                            &serial,
                            &persistence,
                        )
                        .unwrap_or_else(|_| {
                            report(&environment, &fallback_item, "failed", &safety)
                        });
                        (key, vec![result])
                    }
                    ExecutionJob::Many(key, tasks) => {
                        let fallback: Vec<_> = tasks
                            .iter()
                            .map(|task| {
                                report(
                                    &environment,
                                    &task.item,
                                    "failed",
                                    &format!("{:?}", task.resource_type.concurrency_mode)
                                        .to_ascii_lowercase(),
                                )
                            })
                            .collect();
                        let result = execute_many(
                            &environment,
                            tasks,
                            &journal_path,
                            &journal,
                            &serial,
                            &persistence,
                        )
                        .unwrap_or(fallback);
                        (key, result)
                    }
                })
                .collect()
        });
        let mut group_ok: BTreeMap<GroupKey, bool> =
            ready.iter().cloned().map(|key| (key, true)).collect();
        for (key, job_reports) in results {
            for result in job_reports {
                if !matches!(
                    result.outcome.as_str(),
                    "success" | "resumed_success" | "in_sync"
                ) {
                    group_ok.insert(key.clone(), false);
                }
                reports.push(result);
            }
        }
        for (key, ok) in group_ok {
            if ok {
                successful.insert(key);
            } else {
                failed.insert(key);
            }
        }
    }
    let mut deletion_groups: BTreeMap<GroupKey, Vec<PreparedDeletion>> = BTreeMap::new();
    for task in prepared_deletions {
        deletion_groups
            .entry(marker_group(&task.marker))
            .or_default()
            .push(task);
    }
    while !deletion_groups.is_empty() {
        let blocked: Vec<_> = deletion_groups
            .keys()
            .filter(|key| {
                deletion_dependency_groups(&deletion_groups[key])
                    .iter()
                    .any(|dependency| failed.contains(dependency))
            })
            .cloned()
            .collect();
        for key in blocked {
            for task in deletion_groups.remove(&key).unwrap() {
                reports.push(deletion_report(
                    &environment,
                    &task.marker,
                    "blocked_dependency",
                ));
            }
            failed.insert(key);
        }
        let ready: Vec<_> = deletion_groups
            .keys()
            .filter(|key| {
                deletion_dependency_groups(&deletion_groups[key])
                    .iter()
                    .all(|dependency| {
                        successful.contains(dependency) || !deletion_groups.contains_key(dependency)
                    })
            })
            .cloned()
            .collect();
        if ready.is_empty() && !deletion_groups.is_empty() {
            bail!("Resource Type dependency cycle prevents deletion execution");
        }
        for key in ready {
            let mut group_ok = true;
            for task in deletion_groups.remove(&key).unwrap() {
                let fallback = deletion_report(&environment, &task.marker, "failed");
                let result = execute_deletion(&environment, task, &journal_path, &journal, &serial)
                    .unwrap_or(fallback);
                if !matches!(
                    result.outcome.as_str(),
                    "deleted" | "already_absent" | "resumed_success"
                ) {
                    group_ok = false;
                }
                reports.push(result);
            }
            if group_ok {
                successful.insert(key);
            } else {
                failed.insert(key);
            }
        }
    }
    sort_reports(&mut reports);
    stamp_reports(&mut reports, &revision);
    Ok(reports)
}

fn preflight_transformations(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    inventory: &[InventoryEntry],
) -> Result<Vec<PushResult>> {
    let mut reports = Vec::new();
    for item in inventory {
        let target = &project.environments[environment].targets[&item.target];
        let app = load_installed(root, &target.application)?;
        for configured_type in target.sensitive_fields.keys() {
            if !app
                .target_profile
                .resource_types
                .contains_key(configured_type)
            {
                bail!("Target Sensitive Fields reference unknown Resource Type {configured_type}");
            }
        }
        let baseline = load_baseline(&baseline_path(root, environment, &item.target))
            .context("Target Baseline is unavailable; run `taku fetch`")?;
        let effective = from_baseline(&app, target, &baseline)?;
        let resource_type = &effective.resource_types[&item.resource_type];
        if outbound(&item.value, resource_type, None).is_err() {
            reports.push(report(
                environment,
                item,
                "transformation_conflict",
                "preflight",
            ));
        }
    }
    Ok(reports)
}

pub fn push_confirmation_required(
    root: &Path,
    selection: &Selection,
    uncommitted: Option<GitPolicy>,
    untracked: Option<GitPolicy>,
) -> Result<bool> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let inventory = list_inventory(&root, selection)?;
    let markers = deletion_markers(&root, selection)?;
    let marker_paths: Vec<_> = markers
        .iter()
        .map(|(path, _)| path.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    let (has_uncommitted, has_untracked) = selected_git_changes(&root, &inventory, &marker_paths)?;
    let uncommitted_policy = uncommitted
        .or(project.push.uncommitted)
        .unwrap_or(GitPolicy::Confirm);
    let untracked_policy = untracked
        .or(project.push.untracked)
        .unwrap_or(GitPolicy::Block);
    let uncommitted_confirmation =
        confirmation_for_policy("uncommitted", has_uncommitted, uncommitted_policy)?;
    let untracked_confirmation =
        confirmation_for_policy("untracked", has_untracked, untracked_policy)?;
    Ok(uncommitted_confirmation || untracked_confirmation)
}

fn confirmation_for_policy(kind: &str, present: bool, policy: GitPolicy) -> Result<bool> {
    if !present || policy == GitPolicy::Allow {
        return Ok(false);
    }
    match policy {
        GitPolicy::Confirm => Ok(true),
        GitPolicy::Block => {
            bail!("selected {kind} Resources are blocked by Push Git-State Policy")
        }
        GitPolicy::Allow => Ok(false),
    }
}

fn execute_resource(
    root: &Path,
    environment: &str,
    task: PreparedResource,
    journal_path: &Path,
    journal: &Arc<Mutex<PushJournal>>,
    serial: &Arc<Mutex<()>>,
    persistence: &Arc<Mutex<()>>,
) -> Result<PushResult> {
    let key = task.key();
    if journal.lock().unwrap().completed.contains_key(&key) {
        return Ok(report(
            environment,
            &task.item,
            "resumed_success",
            &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
        ));
    }
    let wire = outbound(&task.item.value, &task.resource_type, Some(&task.operation))?;
    let request = || {
        execute(
            &task.target,
            &task.app,
            &task.resource_type,
            &task.operation,
            OperationInput {
                namespace: task.item.namespace.as_deref(),
                id: if task.item.pending {
                    None
                } else {
                    Some(&task.item.id)
                },
                context: Some(&task.item.value),
                body: Some(std::slice::from_ref(&wire)),
            },
            &task.auth,
        )
    };
    let mut attempts = 0;
    let remote = loop {
        attempts += 1;
        let result = if task.operation.concurrency == ConcurrencyClass::Serial {
            let _guard = serial.lock().unwrap();
            request()
        } else {
            request()
        };
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                return Ok(report(
                    environment,
                    &task.item,
                    "transformation_conflict",
                    &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
                ));
            }
        };
        if matches!(result, RemoteResult::Retryable | RemoteResult::Uncertain)
            && task.operation.retry_safe
            && (task.resource_type.write_intent != WriteIntent::Create || !task.item.pending)
            && attempts < 3
        {
            continue;
        }
        break result;
    };
    let outcome = match remote {
        RemoteResult::Success(mut values) => {
            let response_guard = values
                .first()
                .and_then(|value| pointer_string(value, INTERNAL_GUARD_POINTER));
            for value in &mut values {
                remove_pointer(value, INTERNAL_GUARD_POINTER)?;
            }
            if task.item.pending {
                if !task.operation.trustworthy_response
                    || values.len() != 1
                    || pointer_string(&values[0], &task.resource_type.id.pointer).is_none()
                {
                    "creation_conflict"
                } else {
                    let _guard = persistence.lock().unwrap();
                    write_resource(&root.join(&task.item.path), &values[0])?;
                    let mut observation = load_observation(&task.observation_path)?;
                    observation.resources.remove(&task.item.id);
                    save_observation(&task.observation_path, &observation)?;
                    record_success(journal_path, journal, &key)?;
                    "success"
                }
            } else {
                let _guard = persistence.lock().unwrap();
                let mut observation = load_observation(&task.observation_path)?;
                if task.operation.trustworthy_response && values.len() == 1 {
                    if let Some(entry) = observation.resources.get_mut(&task.item.id) {
                        entry.value = Some(values[0].clone());
                        entry.present = true;
                        entry.guard = response_guard;
                    }
                } else {
                    observation.resources.remove(&task.item.id);
                }
                save_observation(&task.observation_path, &observation)?;
                record_success(journal_path, journal, &key)?;
                "success"
            }
        }
        RemoteResult::Conflict => "conflict",
        RemoteResult::Uncertain | RemoteResult::Retryable
            if task.resource_type.write_intent == WriteIntent::Create =>
        {
            "creation_conflict"
        }
        RemoteResult::Uncertain => "uncertain",
        RemoteResult::Retryable | RemoteResult::Failure(_) => "failed",
        RemoteResult::NotFound => "not_found",
    };
    Ok(report(
        environment,
        &task.item,
        outcome,
        &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
    ))
}

fn execute_many(
    environment: &str,
    tasks: Vec<PreparedResource>,
    journal_path: &Path,
    journal: &Arc<Mutex<PushJournal>>,
    serial: &Arc<Mutex<()>>,
    persistence: &Arc<Mutex<()>>,
) -> Result<Vec<PushResult>> {
    let mut reports = Vec::new();
    let mut pending = Vec::new();
    for task in tasks {
        if journal.lock().unwrap().completed.contains_key(&task.key()) {
            reports.push(report(
                environment,
                &task.item,
                "resumed_success",
                &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
            ));
        } else {
            pending.push(task);
        }
    }
    if pending.is_empty() {
        return Ok(reports);
    }
    let wires: Vec<_> = pending
        .iter()
        .map(|task| outbound(&task.item.value, &task.resource_type, Some(&task.operation)))
        .collect::<Result<_>>()?;
    let first = &pending[0];
    let request = || {
        execute(
            &first.target,
            &first.app,
            &first.resource_type,
            &first.operation,
            OperationInput {
                namespace: first.item.namespace.as_deref(),
                id: None,
                context: Some(&first.item.value),
                body: Some(&wires),
            },
            &first.auth,
        )
    };
    let retry_safe = first.operation.retry_safe
        && pending.iter().all(|task| {
            task.resource_type.write_intent != WriteIntent::Create || !task.item.pending
        });
    let mut attempts = 0;
    let remote = loop {
        attempts += 1;
        let result = if first.operation.concurrency == ConcurrencyClass::Serial {
            let _guard = serial.lock().unwrap();
            request()
        } else {
            request()
        };
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                for task in pending {
                    reports.push(report(
                        environment,
                        &task.item,
                        "transformation_conflict",
                        &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
                    ));
                }
                return Ok(reports);
            }
        };
        if matches!(result, RemoteResult::Retryable | RemoteResult::Uncertain)
            && retry_safe
            && attempts < 3
        {
            continue;
        }
        break result;
    };
    let outcome = match remote {
        RemoteResult::Success(_) if pending.iter().any(|task| task.item.pending) => {
            "creation_conflict"
        }
        RemoteResult::Success(_) => {
            let _guard = persistence.lock().unwrap();
            let mut observations = BTreeMap::new();
            for task in &pending {
                if !observations.contains_key(&task.observation_path) {
                    observations.insert(
                        task.observation_path.clone(),
                        load_observation(&task.observation_path)?,
                    );
                }
                observations
                    .get_mut(&task.observation_path)
                    .unwrap()
                    .resources
                    .remove(&task.item.id);
            }
            for (path, observation) in observations {
                save_observation(&path, &observation)?;
            }
            for task in &pending {
                record_success(journal_path, journal, &task.key())?;
            }
            "success"
        }
        RemoteResult::Conflict => "conflict",
        RemoteResult::Uncertain | RemoteResult::Retryable
            if pending
                .iter()
                .any(|task| task.resource_type.write_intent == WriteIntent::Create) =>
        {
            "creation_conflict"
        }
        RemoteResult::Uncertain => "uncertain",
        RemoteResult::Retryable | RemoteResult::Failure(_) => "failed",
        RemoteResult::NotFound => "not_found",
    };
    for task in pending {
        reports.push(report(
            environment,
            &task.item,
            outcome,
            &format!("{:?}", task.resource_type.concurrency_mode).to_ascii_lowercase(),
        ));
    }
    Ok(reports)
}

fn read_current(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    resource_type: &ResourceType,
    auth: &SecretFields,
    namespace: Option<&str>,
    id: &str,
    desired: Option<&serde_json::Value>,
) -> Result<CurrentRemote> {
    let read = resource_type
        .operations
        .read
        .as_ref()
        .context("Write Intent or guarded concurrency requires a Read Operation")?;
    match execute_retry_safe(
        target,
        app,
        resource_type,
        read,
        OperationInput {
            namespace,
            id: Some(id),
            context: desired,
            body: None,
        },
        auth,
    )? {
        RemoteResult::NotFound => Ok(CurrentRemote {
            present: false,
            value: None,
            guard: None,
        }),
        RemoteResult::Success(mut values) if values.len() == 1 => {
            let mut value = values.pop().unwrap();
            let guard = pointer_string(&value, INTERNAL_GUARD_POINTER);
            remove_pointer(&mut value, INTERNAL_GUARD_POINTER)?;
            let guard = guard.or_else(|| Some(hash(&canonical_bytes(&value).ok()?)));
            Ok(CurrentRemote {
                present: true,
                value: Some(value),
                guard,
            })
        }
        RemoteResult::Conflict => bail!("targeted Read Operation reported a conflict for {id}"),
        RemoteResult::Retryable | RemoteResult::Uncertain => {
            bail!("targeted Read Operation was uncertain for {id}")
        }
        RemoteResult::Failure(message) => {
            bail!("targeted Read Operation failed for {id}: {message}")
        }
        RemoteResult::Success(_) => bail!("One Read Operation returned multiple Resources"),
    }
}

fn verify_deletion(task: &PreparedDeletion) -> Result<DeletionVerification> {
    let parameters = marker_parameters(&task.marker);
    match execute_retry_safe(
        &task.target,
        &task.app,
        &task.resource_type,
        &task.read,
        OperationInput {
            namespace: task.marker.namespace.as_deref(),
            id: Some(&task.marker.id),
            context: Some(&parameters),
            body: None,
        },
        &task.auth,
    )? {
        RemoteResult::NotFound => Ok(DeletionVerification::Absent),
        RemoteResult::Success(mut values) if values.len() == 1 => {
            let token = pointer_string(&values[0], INTERNAL_GUARD_POINTER);
            remove_pointer(&mut values[0], INTERNAL_GUARD_POINTER)?;
            let current_guard = token.unwrap_or(hash(&canonical_bytes(&values[0])?));
            if current_guard == task.marker.guard {
                Ok(DeletionVerification::Present)
            } else {
                Ok(DeletionVerification::Conflict)
            }
        }
        RemoteResult::Conflict => Ok(DeletionVerification::Conflict),
        RemoteResult::Retryable | RemoteResult::Uncertain => {
            bail!("Deletion Marker preflight read was uncertain")
        }
        RemoteResult::Failure(message) => {
            bail!("Deletion Marker preflight read failed: {message}")
        }
        RemoteResult::Success(_) => bail!("One Read Operation returned multiple Resources"),
    }
}

fn execute_deletion(
    environment: &str,
    task: PreparedDeletion,
    journal_path: &Path,
    journal: &Arc<Mutex<PushJournal>>,
    serial: &Arc<Mutex<()>>,
) -> Result<PushResult> {
    let key = task.key();
    if journal.lock().unwrap().completed.contains_key(&key) {
        return Ok(PushResult {
            environment: environment.into(),
            target: task.marker.target,
            namespace: task.marker.namespace,
            resource_type: task.marker.resource_type,
            id: task.marker.id,
            outcome: "resumed_success".into(),
            git_revision: String::new(),
            safety: "guarded_deletion_marker".into(),
        });
    }
    let outcome = if task.remote_absent {
        fs::remove_file(&task.path)?;
        record_success(journal_path, journal, &key)?;
        "already_absent"
    } else {
        let parameters = marker_parameters(&task.marker);
        let deleted = {
            let _guard = if task.delete.concurrency == ConcurrencyClass::Serial {
                Some(serial.lock().unwrap())
            } else {
                None
            };
            execute_retry_safe(
                &task.target,
                &task.app,
                &task.resource_type,
                &task.delete,
                OperationInput {
                    namespace: task.marker.namespace.as_deref(),
                    id: Some(&task.marker.id),
                    context: Some(&parameters),
                    body: None,
                },
                &task.auth,
            )?
        };
        match deleted {
            RemoteResult::Success(_) | RemoteResult::NotFound => {
                fs::remove_file(&task.path)?;
                record_success(journal_path, journal, &key)?;
                "deleted"
            }
            RemoteResult::Conflict => "deletion_conflict",
            RemoteResult::Uncertain | RemoteResult::Retryable => "uncertain",
            RemoteResult::Failure(_) => "failed",
        }
    };
    Ok(PushResult {
        environment: environment.into(),
        target: task.marker.target,
        namespace: task.marker.namespace,
        resource_type: task.marker.resource_type,
        id: task.marker.id,
        outcome: outcome.into(),
        git_revision: String::new(),
        safety: "guarded_deletion_marker".into(),
    })
}

fn marker_parameters(marker: &DeletionMarker) -> serde_json::Value {
    serde_json::Value::Object(
        marker
            .parameters
            .iter()
            .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
            .collect(),
    )
}

fn inventory_group(item: &InventoryEntry) -> GroupKey {
    (
        item.target.clone(),
        item.namespace.clone(),
        item.resource_type.clone(),
    )
}

fn marker_group(marker: &DeletionMarker) -> GroupKey {
    (
        marker.target.clone(),
        marker.namespace.clone(),
        marker.resource_type.clone(),
    )
}

fn dependency_group(
    target: &str,
    namespace: Option<&str>,
    resource_types: &BTreeMap<String, ResourceType>,
    dependency: &str,
) -> GroupKey {
    let namespace = resource_types
        .get(dependency)
        .filter(|resource_type| resource_type.namespaced)
        .and(namespace)
        .map(str::to_owned);
    (target.to_owned(), namespace, dependency.to_owned())
}

fn dependency_groups(tasks: &[PreparedResource]) -> Vec<GroupKey> {
    tasks.first().map_or_else(Vec::new, |task| {
        task.resource_type
            .dependencies
            .iter()
            .map(|dependency| {
                dependency_group(
                    &task.item.target,
                    task.item.namespace.as_deref(),
                    &task.effective_resource_types,
                    dependency,
                )
            })
            .collect()
    })
}

fn deletion_dependency_groups(tasks: &[PreparedDeletion]) -> Vec<GroupKey> {
    tasks.first().map_or_else(Vec::new, |task| {
        task.resource_type
            .dependencies
            .iter()
            .map(|dependency| {
                dependency_group(
                    &task.marker.target,
                    task.marker.namespace.as_deref(),
                    &task.effective_resource_types,
                    dependency,
                )
            })
            .collect()
    })
}
fn deletion_report(environment: &str, marker: &DeletionMarker, outcome: &str) -> PushResult {
    PushResult {
        environment: environment.into(),
        target: marker.target.clone(),
        namespace: marker.namespace.clone(),
        resource_type: marker.resource_type.clone(),
        id: marker.id.clone(),
        outcome: outcome.into(),
        git_revision: String::new(),
        safety: "guarded_deletion_marker".into(),
    }
}
fn report(environment: &str, item: &InventoryEntry, outcome: &str, safety: &str) -> PushResult {
    PushResult {
        environment: environment.into(),
        target: item.target.clone(),
        namespace: item.namespace.clone(),
        resource_type: item.resource_type.clone(),
        id: item.id.clone(),
        outcome: outcome.into(),
        git_revision: String::new(),
        safety: safety.into(),
    }
}
fn sort_reports(reports: &mut [PushResult]) {
    reports.sort_by(|a, b| {
        (
            &a.environment,
            &a.target,
            &a.namespace,
            &a.resource_type,
            &a.id,
        )
            .cmp(&(
                &b.environment,
                &b.target,
                &b.namespace,
                &b.resource_type,
                &b.id,
            ))
    });
}
fn stamp_reports(reports: &mut [PushResult], revision: &str) {
    for report in reports {
        report.git_revision = revision.into();
    }
}
fn prepare_journal(
    path: &Path,
    binding: &str,
    total: usize,
    new_plan: bool,
) -> Result<PushJournal> {
    if let Ok(text) = fs::read_to_string(path) {
        let existing: PushJournal = serde_yaml::from_str(&text).context("invalid Push Journal")?;
        if existing.schema_version != crate::SCHEMA_VERSION {
            bail!(
                "unsupported Push Journal schema version {}",
                existing.schema_version
            );
        }
        if existing.binding == binding {
            return Ok(existing);
        }
        if existing.completed.len() < existing.total && !new_plan {
            bail!(
                "an incomplete Push Journal has different inputs; repeat with --new-plan to replace it explicitly"
            );
        }
    }
    let journal = PushJournal {
        schema_version: 1,
        binding: binding.into(),
        total,
        completed: BTreeMap::new(),
    };
    save_journal(path, &journal)?;
    Ok(journal)
}
fn record_success(path: &Path, journal: &Arc<Mutex<PushJournal>>, key: &str) -> Result<()> {
    let mut journal = journal.lock().unwrap();
    journal.completed.insert(key.into(), "success".into());
    save_journal(path, &journal)
}
fn save_journal(path: &Path, journal: &PushJournal) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("next");
    fs::write(&temporary, serde_yaml::to_string(journal)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
fn git_revision(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    } else {
        Ok("unborn".into())
    }
}
fn enforce_git(
    root: &Path,
    items: &[InventoryEntry],
    marker_paths: &[String],
    uncommitted: Option<GitPolicy>,
    untracked: Option<GitPolicy>,
    interactive: bool,
    confirmed: bool,
) -> Result<()> {
    let (has_uncommitted, has_untracked) = selected_git_changes(root, items, marker_paths)?;
    let default_uncommitted = if interactive {
        GitPolicy::Confirm
    } else {
        GitPolicy::Block
    };
    check_policy(
        "uncommitted",
        has_uncommitted,
        uncommitted.unwrap_or(default_uncommitted),
        interactive,
        confirmed,
    )?;
    check_policy(
        "untracked",
        has_untracked,
        untracked.unwrap_or(GitPolicy::Block),
        interactive,
        confirmed,
    )
}

fn selected_git_changes(
    root: &Path,
    items: &[InventoryEntry],
    marker_paths: &[String],
) -> Result<(bool, bool)> {
    let mut paths: Vec<&str> = items.iter().map(|item| item.path.as_str()).collect();
    paths.extend(marker_paths.iter().map(String::as_str));
    if paths.is_empty() {
        return Ok((false, false));
    }
    let mut command = Command::new("git");
    command.args(["status", "--porcelain", "--"]);
    command.args(paths);
    let output = command.current_dir(root).output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok((
        text.lines().any(|line| !line.starts_with("??")),
        text.lines().any(|line| line.starts_with("??")),
    ))
}
fn check_policy(
    kind: &str,
    present: bool,
    policy: GitPolicy,
    interactive: bool,
    confirmed: bool,
) -> Result<()> {
    if !present {
        return Ok(());
    }
    match policy {
        GitPolicy::Allow => Ok(()),
        GitPolicy::Confirm if interactive && confirmed => Ok(()),
        GitPolicy::Confirm if interactive => {
            bail!("selected {kind} Resources require confirmation; repeat with --yes")
        }
        GitPolicy::Confirm | GitPolicy::Block => {
            bail!("selected {kind} Resources are blocked by Push Git-State Policy")
        }
    }
}
