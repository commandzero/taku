use super::{Executor, failed_mutation};
use crate::canonical::{canonical_id, pointer_string, write_canonical_resource};
use crate::observe::{load_observation, save_observation};
use crate::reconcile::PushResult;
use crate::scheduler::PreparedResource;
use crate::transport::{
    INTERNAL_GUARD_POINTER, OperationInput, RemoteResult, execute, outbound, remove_pointer,
};
use anyhow::Result;
use serde_json::Value;

impl Executor<'_> {
    pub(super) fn write_one(&self, task: &PreparedResource) -> Result<PushResult> {
        let key = task.key();
        if self.completed(&key) {
            return Ok(task.report(self.environment, "resumed_success"));
        }
        let remote = request(self, task);
        let outcome = match remote {
            Err(_) => "transformation_conflict",
            Ok(RemoteResult::Success(values)) => self.persist_write(task, &key, values)?,
            Ok(failure) => failed_mutation(failure, task.resource_type.write_intent),
        };
        Ok(task.report(self.environment, outcome))
    }

    fn persist_write(
        &self,
        task: &PreparedResource,
        key: &str,
        mut values: Vec<Value>,
    ) -> Result<&'static str> {
        let response_guard = values
            .first()
            .and_then(|value| pointer_string(value, INTERNAL_GUARD_POINTER));
        for value in &mut values {
            remove_pointer(value, INTERNAL_GUARD_POINTER)?;
        }
        let mapped = single_response(task, &values);
        if task.item.pending
            && mapped.is_none_or(|value| canonical_id(value, &task.resource_type).is_none())
        {
            return Ok("creation_conflict");
        }
        let _guard = self.persistence.lock().unwrap();
        if task.item.pending {
            write_canonical_resource(
                &self.root.join(&task.item.path),
                &values[0],
                &task.resource_type,
            )?;
        }
        update_observation(task, mapped, response_guard)?;
        self.record_success(key)?;
        Ok("success")
    }
}

fn request(executor: &Executor<'_>, task: &PreparedResource) -> Result<RemoteResult> {
    let wire = outbound(&task.item.value, &task.resource_type, Some(&task.operation))?;
    let id = (!task.item.pending).then_some(task.item.id.as_str());
    let input = OperationInput {
        namespace: task.item.namespace.as_deref(),
        id,
        context: Some(&task.item.value),
        body: Some(std::slice::from_ref(&wire)),
        mutation: true,
        metadata_track: task.metadata_track,
        ..Default::default()
    };
    executor.mutation(
        task.operation.concurrency,
        task.operation.retry_safe && task.can_retry(),
        || {
            execute(
                &task.target,
                &task.app,
                &task.resource_type,
                &task.operation,
                input,
                &task.auth,
            )
        },
    )
}

fn single_response<'a>(task: &PreparedResource, values: &'a [Value]) -> Option<&'a Value> {
    match values {
        [value] if task.operation.consumes_response_body() => Some(value),
        _ => None,
    }
}

fn update_observation(
    task: &PreparedResource,
    mapped: Option<&Value>,
    response_guard: Option<String>,
) -> Result<()> {
    let mut observation = load_observation(&task.observation_path)?;
    match mapped.filter(|_| !task.item.pending) {
        Some(value) => {
            if let Some(entry) = observation.resources.get_mut(&task.item.id) {
                entry.value = Some(value.clone());
                entry.present = true;
                entry.guard = response_guard;
            }
        }
        None => {
            observation.resources.remove(&task.item.id);
        }
    }
    save_observation(&task.observation_path, &observation)
}
