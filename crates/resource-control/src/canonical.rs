mod inventory;
mod paths;
mod storage;

pub use inventory::list_inventory;
pub(crate) use inventory::list_inventory_with_resolved;
pub(crate) use paths::{
    is_deletion_marker, reject_symlink_components, resource_path, resource_paths, target_root,
};
pub use storage::parse_resource;
pub(crate) use storage::{remove_canonical_resource, write_canonical_resource};

use crate::{DisplayName, IdScope, ResourceType};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use std::path::{Path, PathBuf};

pub fn resource_directory_in_namespace(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    target: &str,
    namespace: Option<&str>,
    resource_type: &str,
) -> PathBuf {
    paths::resource_directory_in_namespace(
        target_root(root, project, environment, target),
        namespace,
        resource_type,
    )
}

pub fn resource_directories(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    target: &str,
    resource_type: &str,
    namespaced: bool,
) -> Result<Vec<(Option<String>, PathBuf)>> {
    paths::resource_directories(
        target_root(root, project, environment, target),
        resource_type,
        namespaced,
    )
}

#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub environment: Option<String>,
    pub targets: Vec<String>,
    pub namespaces: Vec<String>,
    pub types: Vec<String>,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InventoryEntry {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub pending: bool,
    pub name: String,
    pub display_name: String,
    pub id_scope: IdScope,
    pub path: String,
    #[serde(skip)]
    pub value: Value,
    #[serde(skip)]
    pub display_name_unique: bool,
}

/// The namespace reserved for Taku-managed canonical state.
pub const TAKU_NAMESPACE_POINTER: &str = "/_taku";
/// The stable location of a Resource's identity in its Canonical Representation.
pub const TAKU_ID_POINTER: &str = "/_taku/id";

pub fn pointer_string(value: &Value, pointer: &str) -> Option<String> {
    let value = value.pointer(pointer)?;
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Object(_) | Value::Array(_) => serde_json::to_string(&sort_value(value)).ok(),
        _ => None,
    }
}

/// Resolve a Resource ID from the canonical namespace, with a fallback for
/// older hand-authored Resources that predate `_taku.id`.
pub fn canonical_id(value: &Value, resource_type: &ResourceType) -> Option<String> {
    pointer_string(value, TAKU_ID_POINTER)
        .or_else(|| pointer_string(value, &resource_type.id.pointer))
}

pub fn display_name_value(value: &Value, display_name: &DisplayName) -> Option<String> {
    display_name
        .pointers()
        .find_map(|pointer| pointer_string(value, pointer))
}

pub fn canonical_bytes(value: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&sort_value(value))?)
}

pub fn owned_value(observed: &Value, desired: &Value, mode: crate::MutationMode) -> Value {
    if !matches!(mode, crate::MutationMode::Patch) {
        return observed.clone();
    }
    match desired {
        Value::Object(desired_fields) => {
            let mut owned = serde_json::Map::new();
            for (name, desired_value) in desired_fields {
                let observed_value = observed.get(name).unwrap_or(&Value::Null);
                owned.insert(
                    name.clone(),
                    owned_value(observed_value, desired_value, mode),
                );
            }
            Value::Object(owned)
        }
        _ => observed.clone(),
    }
}

pub fn sort_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(k, _)| *k);
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.clone(), sort_value(v)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(sort_value).collect()),
        other => other.clone(),
    }
}

pub fn safe_filename(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

pub fn short_id(id: &str) -> String {
    let suffix: String = id
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if !suffix.is_empty() && safe_filename(&suffix) == suffix {
        return suffix;
    }
    hex::encode(sha2::Sha256::digest(id.as_bytes()))[..8].into()
}
