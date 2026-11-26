use super::{Preparation, ResourceLocation};
use crate::application::load_installed;
use crate::canonical::{InventoryEntry, canonical_bytes, owned_value, pointer_string};
use crate::observe::{Observation, hash};
use crate::provider::SecretFields;
use crate::resolution::discover;
use crate::transport::{
    INTERNAL_GUARD_POINTER, OperationInput, RemoteResult, execute_retry_safe, remove_pointer,
    without_metadata,
};
use crate::{
    ApplicationDefinition, ConcurrencyMode, MissingPolicy, Operation, ResourceType, TargetConfig,
    WriteIntent,
};
use anyhow::{Context, Result, bail};

use super::super::{PreparedResource, inventory_group, report, safety};

impl Preparation<'_> {
    pub(super) fn resource(&mut self, item: InventoryEntry) -> Result<()> {
        let location = ResourceLocation::for_item(&item);
        let target = self.scope.target(location.target);
        let app = load_installed(self.scope.root, &target.application)?;
        let auth = self.scope.auth(target)?;
        let discovered = match discover(&app, target, &auth) {
            Ok(discovered) => discovered,
            Err(_) => {
                self.fail_resource(&item, "failed", "target_discovery");
                return Ok(());
            }
        };
        let baseline = self.scope.checked_baseline(&location, &discovered)?;
        let resource_type = discovered.resource_types[&item.resource_type].clone();
        let metadata_track = self.bind_resource(&location, &item.value)?;
        self.bind_definition(&app, &resource_type, &baseline)?;
        let (observation_path, observation) = self.observation(
            &location,
            &app,
            "selected Resource is absent from Observed State",
        )?;
        let reading = ResourceRead {
            target,
            app: &app,
            resource_type: &resource_type,
            auth: &auth,
        };
        let current = match reading.current(CurrentInput {
            item: &item,
            observation,
            metadata_track,
        }) {
            Ok(current) => current,
            Err(_) => {
                self.fail_resource(&item, "failed", "targeted_read");
                return Ok(());
            }
        };
        if is_in_sync(&current, &item, &resource_type) {
            self.plan.reports.push(report(
                self.scope.environment,
                &item,
                "in_sync",
                &safety(&resource_type),
            ));
            return Ok(());
        }
        let operation = self.operation_for(&item, &resource_type, &current);
        if let Some(operation) = operation {
            self.plan.resources.push(PreparedResource {
                item,
                target: target.clone(),
                app,
                effective_resource_types: discovered.resource_types,
                resource_type,
                operation,
                auth,
                observation_path,
                metadata_track,
            });
        }
        Ok(())
    }

    fn bind_resource(
        &mut self,
        location: &ResourceLocation<'_>,
        value: &serde_json::Value,
    ) -> Result<bool> {
        let track = self.bind_hints(location)?;
        let digest = hash(&canonical_bytes(value)?);
        self.plan.binding_parts.push(format!(
            "{}:{}:{}:{digest}",
            location.target, location.resource_type, location.id
        ));
        Ok(track)
    }

    fn operation_for(
        &mut self,
        item: &InventoryEntry,
        resource_type: &ResourceType,
        current: &CurrentRemote,
    ) -> Option<Operation> {
        let result = select_operation(resource_type, current, self.scope.missing);
        match result {
            Ok(operation) => Some(operation),
            Err((outcome, None)) => {
                self.fail_resource(item, outcome, &safety(resource_type));
                None
            }
            Err((outcome, Some(safety))) => {
                self.fail_resource(item, outcome, safety);
                None
            }
        }
    }

    fn fail_resource(&mut self, item: &InventoryEntry, outcome: &str, safety: &str) {
        self.plan
            .reports
            .push(report(self.scope.environment, item, outcome, safety));
        self.plan.failed.insert(inventory_group(item));
    }
}

fn is_in_sync(
    current: &CurrentRemote,
    item: &InventoryEntry,
    resource_type: &ResourceType,
) -> bool {
    if !current.present || resource_type.write_intent == WriteIntent::Create {
        return false;
    }
    current.value.as_ref().is_some_and(|value| {
        let desired = without_metadata(&item.value, resource_type).unwrap_or_default();
        let observed = without_metadata(
            &owned_value(value, &item.value, resource_type.mutation_mode),
            resource_type,
        )
        .unwrap_or_default();
        hash(&canonical_bytes(&observed).unwrap_or_default())
            == hash(&canonical_bytes(&desired).unwrap_or_default())
    })
}

type PrepareFailure = (&'static str, Option<&'static str>);

fn select_operation(
    resource_type: &ResourceType,
    current: &CurrentRemote,
    missing: Option<MissingPolicy>,
) -> std::result::Result<Operation, PrepareFailure> {
    check_missing(resource_type, current.present, missing)?;
    let operations = &resource_type.operations;
    let operation = match (resource_type.write_intent, current.present) {
        (WriteIntent::Create, true) => return Err(("creation_conflict", Some("existence_guard"))),
        (WriteIntent::Update, false) => return Err(("presence_conflict", Some("existence_guard"))),
        (WriteIntent::Create, false) => operations.create.as_ref(),
        (WriteIntent::Update, true) => operations.update.as_ref(),
        (WriteIntent::Upsert, true) => operations.upsert.as_ref().or(operations.update.as_ref()),
        (WriteIntent::Upsert, false) => operations.upsert.as_ref().or(operations.create.as_ref()),
    };
    let mut operation = operation.cloned().ok_or(("failed", Some("write_intent")))?;
    if resource_type.concurrency_mode == ConcurrencyMode::Guarded && current.present {
        set_guard(&mut operation, current.guard.as_deref())?;
    }
    Ok(operation)
}

fn check_missing(
    resource_type: &ResourceType,
    present: bool,
    missing: Option<MissingPolicy>,
) -> std::result::Result<(), PrepareFailure> {
    if present || resource_type.write_intent == WriteIntent::Create {
        return Ok(());
    }
    match missing
        .or(resource_type.missing.push)
        .unwrap_or(MissingPolicy::Conflict)
    {
        MissingPolicy::Conflict => Err(("presence_conflict", None)),
        MissingPolicy::Delete => Err(("failed", Some("missing_policy"))),
        MissingPolicy::Restore => Ok(()),
    }
}

fn set_guard(
    operation: &mut Operation,
    guard: Option<&str>,
) -> std::result::Result<(), PrepareFailure> {
    let Some((header, guard)) = operation.guard_header.as_ref().zip(guard) else {
        return Err(("failed", Some("concurrency_guard")));
    };
    operation.headers.insert(header.clone(), guard.to_owned());
    Ok(())
}

struct CurrentInput<'a> {
    item: &'a InventoryEntry,
    observation: &'a Observation,
    metadata_track: bool,
}

struct CurrentRemote {
    present: bool,
    value: Option<serde_json::Value>,
    guard: Option<String>,
}

struct ResourceRead<'a> {
    target: &'a TargetConfig,
    app: &'a ApplicationDefinition,
    resource_type: &'a ResourceType,
    auth: &'a SecretFields,
}

impl ResourceRead<'_> {
    fn current(&self, input: CurrentInput<'_>) -> Result<CurrentRemote> {
        if self.resource_type.write_intent == WriteIntent::Upsert
            && self.resource_type.concurrency_mode != ConcurrencyMode::Guarded
        {
            return Ok(CurrentRemote {
                present: input.observation.present,
                value: input.observation.value.clone(),
                guard: input.observation.guard.clone(),
            });
        }
        self.read(OperationInput {
            namespace: input.item.namespace.as_deref(),
            id: Some(&input.item.id),
            context: Some(&input.item.value),
            metadata_track: input.metadata_track,
            ..Default::default()
        })
    }

    fn read(&self, input: OperationInput<'_>) -> Result<CurrentRemote> {
        let id = input.id.context("targeted Read Operation requires an ID")?;
        let read = self
            .resource_type
            .operations
            .read
            .as_ref()
            .context("Write Intent or guarded concurrency requires a Read Operation")?;
        match execute_retry_safe(
            self.target,
            self.app,
            self.resource_type,
            read,
            input,
            self.auth,
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
}
