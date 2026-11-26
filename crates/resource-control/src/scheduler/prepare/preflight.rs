use crate::application::load_installed;
use crate::canonical::InventoryEntry;
use crate::observe::{cache_path, load_observation};
use crate::reconcile::PushResult;
use crate::resolution::from_baseline;
use crate::transport::outbound;
use crate::{Operation, ResourceType, WriteIntent};
use anyhow::{Result, bail};
use std::path::Path;

use super::{load_required_baseline, report};

pub(in crate::scheduler) fn preflight_transformations(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    inventory: &[InventoryEntry],
) -> Result<Vec<PushResult>> {
    let mut reports = Vec::new();
    for item in inventory {
        let target = &project.environments[environment].targets[&item.target];
        let app = load_installed(root, &target.application)?;
        let baseline = load_required_baseline(root, environment, &item.target)?;
        let effective = from_baseline(&app, target, &baseline)?;
        for configured_type in target.sensitive_fields.keys() {
            if !effective.resource_types.contains_key(configured_type) {
                bail!("Target Sensitive Fields reference unknown Resource Type {configured_type}");
            }
        }
        let resource_type = &effective.resource_types[&item.resource_type];
        let operation = preflight_operation(root, environment, item, resource_type)?;
        if outbound(&item.value, resource_type, operation).is_err() {
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

fn preflight_operation<'a>(
    root: &Path,
    environment: &str,
    item: &InventoryEntry,
    resource_type: &'a ResourceType,
) -> Result<Option<&'a Operation>> {
    let operations = &resource_type.operations;
    match resource_type.write_intent {
        WriteIntent::Create => Ok(operations.create.as_ref()),
        WriteIntent::Update => Ok(operations.update.as_ref()),
        WriteIntent::Upsert => {
            if let Some(upsert) = &operations.upsert {
                return Ok(Some(upsert));
            }
            let observation = load_observation(&cache_path(
                root,
                environment,
                &item.target,
                item.namespace.as_deref(),
                &item.resource_type,
            ))?;
            let present = observation
                .resources
                .get(&item.id)
                .is_some_and(|resource| resource.present);
            Ok(if present {
                operations.update.as_ref()
            } else {
                operations.create.as_ref()
            })
        }
    }
}
