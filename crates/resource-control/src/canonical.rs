use crate::application::load_installed;
use crate::project::current_environment;
use crate::{DisplayNameStrategy, IdScope, Project, RepositoryLayout, git_root, load_project};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub environment: Option<String>,
    pub targets: Vec<String>,
    pub types: Vec<String>,
    pub ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InventoryEntry {
    pub environment: String,
    pub target: String,
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

pub fn list_inventory(root: &Path, selection: &Selection) -> Result<Vec<InventoryEntry>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let env = &project.environments[&environment];
    let mut result = Vec::new();
    for (target_name, target) in &env.targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        for (type_name, resource_type) in &app.target_profile.resource_types {
            if !selection.types.is_empty() && !selection.types.contains(type_name) {
                continue;
            }
            let directory =
                resource_directory(&root, &project, &environment, target_name, type_name);
            if !directory.exists() {
                continue;
            }
            reject_symlink_components(&root, &directory)?;
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_symlink() {
                    bail!(
                        "symlinked Resource input is not allowed: {}",
                        path.display()
                    );
                }
                if !metadata.is_file() || is_deletion_marker(&path) {
                    continue;
                }
                let encoded_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("Resource filename is not valid UTF-8")?;
                let decoded = urlencoding::decode(encoded_name)
                    .context("Resource filename contains invalid escapes")?;
                if decoded.contains("..") || decoded.contains('/') || decoded.contains('\\') {
                    bail!("Resource filename contains a traversal escape: {encoded_name}");
                }
                if !matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("json" | "json5" | "yaml" | "yml")
                ) {
                    continue;
                }
                let value = parse_resource(&path)?;
                let sensitive_fields = resource_type
                    .sensitive_fields
                    .iter()
                    .chain(target.sensitive_fields.get(type_name).into_iter().flatten());
                for pointer in sensitive_fields {
                    if crate::model::sensitive_field_conflicts(resource_type, pointer) {
                        bail!(
                            "Sensitive Field {pointer} overlaps required canonical state for {type_name}"
                        );
                    }
                    if value.pointer(pointer).is_some() {
                        bail!(
                            "Canonical Resource {} contains Sensitive Field {pointer}",
                            path.display()
                        );
                    }
                }
                let extracted_id = pointer_string(&value, &resource_type.id.pointer);
                let pending = extracted_id.is_none()
                    && resource_type.id.scope == IdScope::Target
                    && resource_type.write_intent == crate::WriteIntent::Create;
                let id = if pending {
                    format!("pending:{}", path.strip_prefix(&root).unwrap().display())
                } else {
                    extracted_id.with_context(|| {
                        format!(
                            "Resource {} has no string ID at {}",
                            path.display(),
                            resource_type.id.pointer
                        )
                    })?
                };
                if !selection.ids.is_empty() && !selection.ids.contains(&id) {
                    continue;
                }
                let name = pointer_string(&value, &resource_type.display_name.pointer)
                    .unwrap_or_else(|| id.clone());
                let display_name = match resource_type.display_name.strategy {
                    DisplayNameStrategy::Id => id.clone(),
                    DisplayNameStrategy::Name => name.clone(),
                    DisplayNameStrategy::NameId => format!("{}-{}", name, short_id(&id)),
                };
                result.push(InventoryEntry {
                    environment: environment.clone(),
                    target: target_name.clone(),
                    resource_type: type_name.clone(),
                    id,
                    pending,
                    name,
                    display_name,
                    id_scope: resource_type.id.scope,
                    path: path.strip_prefix(&root).unwrap().display().to_string(),
                    value,
                    display_name_unique: resource_type.display_name.unique,
                });
            }
        }
    }
    result.sort_by(|a, b| {
        (&a.environment, &a.target, &a.resource_type, &a.id).cmp(&(
            &b.environment,
            &b.target,
            &b.resource_type,
            &b.id,
        ))
    });
    let mut names = std::collections::BTreeSet::new();
    for item in &result {
        if item.display_name_unique
            && !names.insert((
                &item.environment,
                &item.target,
                &item.resource_type,
                &item.display_name,
            ))
        {
            bail!("Display Name collision for {}", item.display_name);
        }
    }
    Ok(result)
}

pub fn resource_directory(
    root: &Path,
    project: &Project,
    environment: &str,
    target: &str,
    resource_type: &str,
) -> PathBuf {
    match project.layout {
        RepositoryLayout::Single => root.join(target).join(resource_type),
        RepositoryLayout::Multi => root.join(environment).join(target).join(resource_type),
    }
}

pub fn parse_resource(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read Resource {}", path.display()))?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("yaml" | "yml") => serde_yaml::from_str(&text).context("invalid YAML Resource"),
        _ => json5::from_str(&normalize_triple_quotes(&text)).context("invalid JSON Resource"),
    }
}

pub fn write_resource(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = if matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    ) {
        serde_yaml::to_string(value)?
    } else {
        serde_json::to_string_pretty(value)? + "\n"
    };
    let temporary = path.with_extension("taku.tmp");
    fs::write(&temporary, text)?;
    fs::rename(temporary, path)?;
    Ok(())
}

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
    hex::encode(sha2::Sha256::digest(id.as_bytes()))[..8].into()
}

fn is_deletion_marker(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".delete.yml"))
}

pub(crate) fn reject_symlink_components(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .context("Resource path escapes Project")?;
    let mut current = root.to_owned();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("invalid Resource path");
        }
        current.push(component);
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(
                "symlinked Resource directory is not allowed: {}",
                current.display()
            );
        }
    }
    Ok(())
}

fn normalize_triple_quotes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("\"\"\"") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        if let Some(end) = after.find("\"\"\"") {
            output.push_str(&serde_json::to_string(&after[..end]).unwrap());
            rest = &after[end + 3..];
        } else {
            output.push_str(&rest[start..]);
            return output;
        }
    }
    output.push_str(rest);
    output
}

use sha2::Digest;
