use super::super::{PreparedDeletion, deletion_report, marker_group};
use super::{Preparation, ResourceLocation};
use crate::application::load_installed;
use crate::canonical::{canonical_bytes, pointer_string};
use crate::lifecycle::DeletionMarker;
use crate::observe::hash;
use crate::resolution::discover;
use crate::transport::{
    INTERNAL_GUARD_POINTER, OperationInput, RemoteResult, execute_retry_safe, remove_pointer,
};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

impl Preparation<'_> {
    pub(super) fn deletion(&mut self, path: PathBuf, marker: DeletionMarker) -> Result<()> {
        let location = ResourceLocation::for_marker(&marker);
        let target = self.scope.target(location.target);
        let app = load_installed(self.scope.root, &target.application)?;
        let metadata_track = self.bind_hints(&location)?;
        self.observation(
            &location,
            &app,
            "selected deletion marker is absent from Observed State",
        )?;
        let auth = self.scope.auth(target)?;
        let discovered = match discover(&app, target, &auth) {
            Ok(discovered) => discovered,
            Err(_) => {
                self.fail_deletion(&marker, "failed");
                return Ok(());
            }
        };
        let baseline = self.scope.checked_baseline(&location, &discovered)?;
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
        self.plan.binding_parts.push(format!(
            "{}:{}:{}:{}",
            marker.target, marker.resource_type, marker.id, marker.guard
        ));
        self.bind_definition(&app, &resource_type, &baseline)?;
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
            metadata_track,
            remote_absent: false,
        };
        match verify_deletion(&prepared) {
            Err(_) => self.fail_deletion(&prepared.marker, "failed"),
            Ok(DeletionVerification::Present) => self.plan.deletions.push(prepared),
            Ok(DeletionVerification::Absent) => {
                prepared.remote_absent = true;
                self.plan.deletions.push(prepared);
            }
            Ok(DeletionVerification::Conflict) => {
                self.fail_deletion(&prepared.marker, "deletion_conflict")
            }
        }
        Ok(())
    }

    fn fail_deletion(&mut self, marker: &DeletionMarker, outcome: &str) {
        self.plan.failed.insert(marker_group(marker));
        self.plan
            .reports
            .push(deletion_report(self.scope.environment, marker, outcome));
    }
}

enum DeletionVerification {
    Present,
    Absent,
    Conflict,
}

fn verify_deletion(task: &PreparedDeletion) -> Result<DeletionVerification> {
    let parameters = task.marker_parameters();
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
            resource_ids: None,
            mutation: false,
            metadata_track: task.metadata_track,
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
