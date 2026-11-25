use super::Executor;
use crate::ConcurrencyClass;
use crate::reconcile::PushResult;
use crate::scheduler::{PreparedDeletion, deletion_report};
use crate::transport::{OperationInput, RemoteResult, execute_retry_safe};
use anyhow::Result;
use std::fs;

impl Executor<'_> {
    pub(super) fn delete(&self, task: &PreparedDeletion) -> Result<PushResult> {
        let key = task.key();
        let outcome = if self.completed(&key) {
            "resumed_success"
        } else if task.remote_absent {
            self.consume_marker(task, &key)?;
            "already_absent"
        } else {
            self.delete_remote(task, &key)?
        };
        Ok(deletion_report(self.environment, &task.marker, outcome))
    }

    fn delete_remote(&self, task: &PreparedDeletion, key: &str) -> Result<&'static str> {
        let parameters = task.marker_parameters();
        let result = {
            let _guard = (task.delete.concurrency == ConcurrencyClass::Serial)
                .then(|| self.serial.lock().unwrap());
            execute_retry_safe(
                &task.target,
                &task.app,
                &task.resource_type,
                &task.delete,
                OperationInput {
                    namespace: task.marker.namespace.as_deref(),
                    id: Some(&task.marker.id),
                    context: Some(&parameters),
                    mutation: true,
                    ..Default::default()
                },
                &task.auth,
            )?
        };
        match result {
            RemoteResult::Success(_) | RemoteResult::NotFound => {
                self.consume_marker(task, key)?;
                Ok("deleted")
            }
            RemoteResult::Conflict => Ok("deletion_conflict"),
            RemoteResult::Uncertain | RemoteResult::Retryable => Ok("uncertain"),
            RemoteResult::Failure(_) => Ok("failed"),
        }
    }

    fn consume_marker(&self, task: &PreparedDeletion, key: &str) -> Result<()> {
        fs::remove_file(&task.path)?;
        self.record_success(key)
    }
}
