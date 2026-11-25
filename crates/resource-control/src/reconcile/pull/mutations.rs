use crate::ResourceType;
use crate::canonical::{parse_resource, remove_canonical_resource, write_canonical_resource};
use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) type ResourceMutation = (PathBuf, Option<Value>, ResourceType);

pub(super) fn apply(mutations: &[ResourceMutation]) -> Result<()> {
    let mut snapshots: Vec<(&Path, Option<Value>, &ResourceType)> = Vec::new();
    for (path, value, resource_type) in mutations {
        let before = snapshot(path, resource_type)?;
        if let Err(error) = write(path, value.as_ref(), resource_type) {
            for (path, contents, resource_type) in snapshots.into_iter().rev() {
                restore(path, contents.as_ref(), resource_type);
            }
            return Err(error);
        }
        snapshots.push((path, before, resource_type));
    }
    Ok(())
}

fn snapshot(path: &Path, resource_type: &ResourceType) -> Result<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    match &resource_type.filesystem {
        Some(projection) => crate::projection::merge(path, projection).map(Some),
        None => parse_resource(path).map(Some),
    }
}

fn write(path: &Path, value: Option<&Value>, resource_type: &ResourceType) -> Result<()> {
    match value {
        Some(value) => write_canonical_resource(path, value, resource_type),
        None => remove_canonical_resource(path),
    }
}

fn restore(path: &Path, value: Option<&Value>, resource_type: &ResourceType) {
    if value.is_some() || path.exists() {
        let _ = write(path, value, resource_type);
    }
}
