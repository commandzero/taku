mod discovery;
mod fetch;
mod query;

pub use fetch::fetch;
pub use query::{remote_list, remote_list_read_only, remote_resource_types_read_only};

use crate::SCHEMA_VERSION;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct FetchResult {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RemoteEntry {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub name: String,
    pub tracked: bool,
    #[serde(skip)]
    pub value: Value,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RemoteResourceType {
    pub name: String,
    pub namespaced: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFile {
    pub schema_version: u32,
    pub observed_at: DateTime<Utc>,
    pub binding: String,
    #[serde(default)]
    pub application_version: String,
    pub catalog_version: String,
    #[serde(default)]
    pub definitions: BTreeMap<String, String>,
    pub resources: BTreeMap<String, Observation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub local_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_value: Option<Value>,
    pub path: String,
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub requires_pull: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint_binding: Option<String>,
}

pub fn is_transformation_conflict(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains("Transformation Conflict"))
}

pub fn cache_path(
    root: &Path,
    environment: &str,
    target: &str,
    namespace: Option<&str>,
    resource_type: &str,
) -> PathBuf {
    let target_root = root.join(".taku/cache").join(environment).join(target);
    match namespace {
        Some(namespace) => target_root
            .join(namespace)
            .join(format!("{resource_type}.yaml")),
        None => target_root.join(format!("{resource_type}.yaml")),
    }
}
pub fn load_observation(path: &Path) -> Result<ObservationFile> {
    let file: ObservationFile = serde_yaml::from_str(
        &fs::read_to_string(path)
            .context("Observed State Cache is unavailable; run `taku fetch`")?,
    )
    .context("invalid Observed State Cache")?;
    if file.schema_version != SCHEMA_VERSION {
        bail!("unsupported Observed State schema version");
    }
    Ok(file)
}
pub fn save_observation(path: &Path, file: &ObservationFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_yaml::to_string(file)?)?;
    Ok(())
}
pub fn binding(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    target: &str,
    app: &crate::ApplicationDefinition,
    namespace: Option<&str>,
    resource_type: &str,
) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(serde_yaml::to_string(project)?);
    digest.update(environment);
    digest.update(target);
    digest.update(serde_yaml::to_string(app)?);
    for (major, catalog) in &app.catalogs {
        digest.update(major.to_be_bytes());
        digest.update(serde_yaml::to_string(catalog)?);
    }
    let hints =
        crate::hints::resolve(root, project, environment, target, namespace, resource_type)?;
    digest.update(hints.binding_material(root)?);
    Ok(hex::encode(digest.finalize()))
}
pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
