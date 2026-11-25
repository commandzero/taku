mod execute;
mod git_state;
mod journal;
mod prepare;

pub use git_state::push_confirmation_required;
use git_state::{GitPolicyCheck, enforce_git, git_revision};

use crate::canonical::{InventoryEntry, Selection, list_inventory};
use crate::lifecycle::{DeletionMarker, deletion_markers};
use crate::project::current_environment;
use crate::provider::SecretFields;
use crate::reconcile::PushResult;
use crate::{
    ApplicationDefinition, GitPolicy, MissingPolicy, Operation, ResourceType, TargetConfig,
    WriteIntent, git_root, load_project,
};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    metadata_track: bool,
}
impl PreparedResource {
    fn group(&self) -> GroupKey {
        inventory_group(&self.item)
    }
    fn report(&self, environment: &str, outcome: &str) -> PushResult {
        report(
            environment,
            &self.item,
            outcome,
            &safety(&self.resource_type),
        )
    }

    fn can_retry(&self) -> bool {
        self.resource_type.write_intent != WriteIntent::Create || !self.item.pending
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
    metadata_track: bool,
    remote_absent: bool,
}
impl PreparedDeletion {
    fn marker_parameters(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.marker
                .parameters
                .iter()
                .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
                .collect(),
        )
    }

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
        prepare::preflight_transformations(&root, &project, &environment, &inventory)?;
    if !preflight_conflicts.is_empty() {
        let revision = git_revision(&root)?;
        sort_reports(&mut preflight_conflicts);
        stamp_reports(&mut preflight_conflicts, &revision);
        return Ok(preflight_conflicts);
    }
    let markers = deletion_markers(&root, selection)?;
    let marker_paths = git_state::marker_paths(&root, &markers);
    enforce_git(
        &root,
        &inventory,
        &marker_paths,
        GitPolicyCheck {
            uncommitted: uncommitted.or(project.push.uncommitted),
            untracked: untracked.or(project.push.untracked),
            interactive,
            confirmed,
        },
    )?;
    let revision = git_revision(&root)?;
    let scope = prepare::PushScope {
        root: &root,
        project: &project,
        environment: &environment,
        cli_provider,
        missing,
    };
    let mut plan = prepare::prepare(&scope, inventory, markers, &revision)?;
    if dry_run {
        plan.add_dry_run_reports();
    } else {
        execute::run(&scope, &mut plan, new_plan)?;
    }
    sort_reports(&mut plan.reports);
    stamp_reports(&mut plan.reports, &revision);
    Ok(plan.reports)
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

fn deletion_report(environment: &str, marker: &DeletionMarker, outcome: &str) -> PushResult {
    make_report(
        environment,
        (
            &marker.target,
            marker.namespace.as_deref(),
            &marker.resource_type,
            &marker.id,
        ),
        outcome,
        "guarded_deletion_marker",
    )
}
fn report(environment: &str, item: &InventoryEntry, outcome: &str, safety: &str) -> PushResult {
    make_report(
        environment,
        (
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
            &item.id,
        ),
        outcome,
        safety,
    )
}

fn make_report(
    environment: &str,
    (target, namespace, resource_type, id): (&str, Option<&str>, &str, &str),
    outcome: &str,
    safety: &str,
) -> PushResult {
    PushResult {
        environment: environment.into(),
        target: target.into(),
        namespace: namespace.map(str::to_owned),
        resource_type: resource_type.into(),
        id: id.into(),
        outcome: outcome.into(),
        git_revision: String::new(),
        safety: safety.into(),
    }
}

fn safety(resource_type: &ResourceType) -> String {
    format!("{:?}", resource_type.concurrency_mode).to_ascii_lowercase()
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
