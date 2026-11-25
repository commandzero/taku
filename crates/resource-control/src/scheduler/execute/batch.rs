use super::{Executor, failed_mutation};
use crate::WriteIntent;
use crate::observe::{load_observation, save_observation};
use crate::reconcile::PushResult;
use crate::scheduler::PreparedResource;
use crate::transport::{OperationInput, RemoteResult, execute, outbound};
use anyhow::Result;
use std::collections::{BTreeMap, btree_map::Entry};

impl Executor<'_> {
    pub(super) fn write_batch(&self, tasks: Vec<PreparedResource>) -> Result<Vec<PushResult>> {
        let mut reports = Vec::new();
        let mut pending = Vec::new();
        for task in tasks {
            if self.completed(&task.key()) {
                reports.push(task.report(self.environment, "resumed_success"));
            } else {
                pending.push(task);
            }
        }
        if pending.is_empty() {
            return Ok(reports);
        }
        let remote = batch_request(self, &pending);
        let outcome = self.batch_outcome(remote, &pending)?;
        reports.extend(
            pending
                .iter()
                .map(|task| task.report(self.environment, outcome)),
        );
        Ok(reports)
    }

    fn batch_outcome(
        &self,
        remote: Result<RemoteResult>,
        pending: &[PreparedResource],
    ) -> Result<&'static str> {
        match remote {
            Err(_) => Ok("transformation_conflict"),
            Ok(RemoteResult::Success(_)) if pending.iter().any(|task| task.item.pending) => {
                Ok("creation_conflict")
            }
            Ok(RemoteResult::Success(_)) => {
                self.persist_batch(pending)?;
                Ok("success")
            }
            Ok(failure) => Ok(failed_mutation(failure, batch_intent(pending))),
        }
    }

    fn persist_batch(&self, pending: &[PreparedResource]) -> Result<()> {
        let _guard = self.persistence.lock().unwrap();
        let mut observations = BTreeMap::new();
        for task in pending {
            let observation = match observations.entry(&task.observation_path) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(load_observation(&task.observation_path)?),
            };
            observation.resources.remove(&task.item.id);
        }
        for (path, observation) in observations {
            save_observation(path, &observation)?;
        }
        for task in pending {
            self.record_success(&task.key())?;
        }
        Ok(())
    }
}

fn batch_intent(tasks: &[PreparedResource]) -> WriteIntent {
    if tasks
        .iter()
        .any(|task| task.resource_type.write_intent == WriteIntent::Create)
    {
        WriteIntent::Create
    } else {
        tasks[0].resource_type.write_intent
    }
}

fn batch_request(executor: &Executor<'_>, tasks: &[PreparedResource]) -> Result<RemoteResult> {
    let wires = tasks
        .iter()
        .map(|task| outbound(&task.item.value, &task.resource_type, Some(&task.operation)))
        .collect::<Result<Vec<_>>>()?;
    let ids = tasks
        .iter()
        .map(|task| task.item.id.clone())
        .collect::<Vec<_>>();
    let first = &tasks[0];
    let input = OperationInput {
        namespace: first.item.namespace.as_deref(),
        context: Some(&first.item.value),
        body: Some(&wires),
        resource_ids: Some(&ids),
        mutation: true,
        metadata_track: first.metadata_track,
        ..Default::default()
    };
    let retry_safe = first.operation.retry_safe && tasks.iter().all(PreparedResource::can_retry);
    executor.mutation(first.operation.concurrency, retry_safe, || {
        execute(
            &first.target,
            &first.app,
            &first.resource_type,
            &first.operation,
            input,
            &first.auth,
        )
    })
}
