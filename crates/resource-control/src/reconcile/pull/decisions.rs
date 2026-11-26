use crate::canonical::{canonical_bytes, owned_value};
use crate::observe::{Observation, hash};
use crate::{MissingPolicy, ResourceType};
use anyhow::{Result, bail};
use serde_json::Value;

pub(super) enum ResourceOutcome {
    Unchanged,
    LocalOnly,
    Pull(Value),
    Delete,
    Conflict(&'static str),
}

impl ResourceOutcome {
    pub(super) fn report(&self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::LocalOnly => "local_only",
            Self::Pull(_) => "pulled",
            Self::Delete => "deleted",
            Self::Conflict(outcome) => outcome,
        }
    }
}

pub(super) struct PullPolicy {
    pub(super) missing: Option<MissingPolicy>,
    pub(super) metadata_track: bool,
}

pub(super) fn reconcile_resource(
    desired: &Value,
    resource_type: &ResourceType,
    resource: &mut Observation,
    policy: PullPolicy,
) -> Result<ResourceOutcome> {
    let current_hash = hash(&canonical_bytes(desired)?);
    let outcome = decide(
        (desired, &current_hash),
        resource_type.mutation_mode,
        resource,
        policy.missing.or(resource_type.missing.pull),
    )?;
    if refresh_desired(resource, &outcome, policy.metadata_track) {
        resource.local_hash = current_hash;
        resource.local_value = Some(desired.clone());
        return Ok(ResourceOutcome::Pull(desired.clone()));
    }
    Ok(outcome)
}

fn refresh_desired(
    resource: &Observation,
    outcome: &ResourceOutcome,
    metadata_track: bool,
) -> bool {
    if metadata_track || !resource.requires_pull {
        return false;
    }
    matches!(
        outcome,
        ResourceOutcome::Unchanged | ResourceOutcome::LocalOnly
    )
}

fn decide(
    (desired, current_hash): (&Value, &str),
    mode: crate::MutationMode,
    resource: &mut Observation,
    missing: Option<MissingPolicy>,
) -> Result<ResourceOutcome> {
    if !resource.present {
        return reconcile_missing(missing);
    }
    let remote = resource.value.as_ref().unwrap();
    let remote_at_current = owned_value(remote, desired, mode);
    let Some(base) = &resource.local_value else {
        return reconcile_by_hash(resource, remote_at_current, current_hash);
    };
    let remote_at_base = owned_value(remote, base, mode);
    let base_bytes = canonical_bytes(base)?;
    let current_bytes = canonical_bytes(desired)?;
    let local_changed = current_bytes != base_bytes;
    let remote_changed = canonical_bytes(&remote_at_base)? != base_bytes;
    match (local_changed, remote_changed) {
        (false, false) => Ok(ResourceOutcome::Unchanged),
        (true, false) => Ok(ResourceOutcome::LocalOnly),
        (false, true) => {
            resource.local_hash = hash(&canonical_bytes(&remote_at_current)?);
            resource.local_value = Some(remote_at_current.clone());
            Ok(ResourceOutcome::Pull(remote_at_current))
        }
        (true, true) if canonical_bytes(&remote_at_current)? == current_bytes => {
            resource.local_hash = current_hash.into();
            resource.local_value = Some(desired.clone());
            Ok(ResourceOutcome::Unchanged)
        }
        (true, true) => Ok(ResourceOutcome::Conflict("pull_conflict")),
    }
}

fn reconcile_missing(missing: Option<MissingPolicy>) -> Result<ResourceOutcome> {
    match missing.unwrap_or(MissingPolicy::Conflict) {
        MissingPolicy::Delete => Ok(ResourceOutcome::Delete),
        MissingPolicy::Conflict => Ok(ResourceOutcome::Conflict("presence_conflict")),
        MissingPolicy::Restore => bail!("restore is not valid for Pull"),
    }
}

fn reconcile_by_hash(
    resource: &mut Observation,
    observed: Value,
    current_hash: &str,
) -> Result<ResourceOutcome> {
    let remote_hash = hash(&canonical_bytes(&observed)?);
    if current_hash == remote_hash {
        return Ok(ResourceOutcome::Unchanged);
    }
    if current_hash == resource.local_hash {
        resource.local_hash = remote_hash;
        return Ok(ResourceOutcome::Pull(observed));
    }
    if remote_hash == resource.local_hash {
        return Ok(ResourceOutcome::LocalOnly);
    }
    Ok(ResourceOutcome::Conflict("pull_conflict"))
}
