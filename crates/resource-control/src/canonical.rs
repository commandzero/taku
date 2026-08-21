use crate::application::load_installed;
use crate::project::current_environment;
use crate::resolution::ResolvedApplication;
use crate::resolution::{baseline_path, for_local_use, load_baseline};
use crate::{
    DisplayName, DisplayNameStrategy, IdScope, Project, RepositoryLayout, ResourceType, git_root,
    load_project,
};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use std::fs;
use std::path::{Component, Path, PathBuf};

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

pub fn list_inventory(root: &Path, selection: &Selection) -> Result<Vec<InventoryEntry>> {
    list_inventory_with_resolved(root, selection, None)
}

pub(crate) fn list_inventory_with_resolved(
    root: &Path,
    selection: &Selection,
    resolved_overrides: Option<&std::collections::BTreeMap<String, ResolvedApplication>>,
) -> Result<Vec<InventoryEntry>> {
    for namespace in &selection.namespaces {
        validate_namespace(namespace)?;
    }
    let root = git_root(root)?;
    let project = load_project(&root)?;
    crate::hints::validate_project_placement(&root, &project)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let env = &project.environments[&environment];
    let mut result = Vec::new();
    for (target_name, target) in &env.targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        let baseline = load_baseline(&baseline_path(&root, &environment, target_name)).ok();
        let resolved_fallback;
        let resolved = if let Some(resolved) =
            resolved_overrides.and_then(|resolved| resolved.get(target_name))
        {
            resolved
        } else {
            resolved_fallback = for_local_use(&app, target, baseline.as_ref())?;
            &resolved_fallback
        };
        crate::hints::validate_target_tree(
            &root,
            &project,
            &environment,
            target_name,
            &resolved.resource_types,
        )?;
        if let Some(baseline) = &baseline {
            let version = semver::Version::parse(&baseline.application_version)?;
            let catalog = &app.catalogs[&version.major];
            for (type_name, definitions) in &catalog.resource_types {
                if resolved.resource_types.contains_key(type_name) {
                    continue;
                }
                let mut layouts: Vec<bool> = definitions
                    .iter()
                    .map(|definition| definition.namespaced)
                    .collect();
                layouts.sort_unstable();
                layouts.dedup();
                for namespaced in layouts {
                    for (_, directory) in resource_directories(
                        &root,
                        &project,
                        &environment,
                        target_name,
                        type_name,
                        namespaced,
                    )? {
                        if directory_has_recognized_input(&directory, definitions)? {
                            bail!(
                                "Resource Type {type_name} is unavailable for Application Version {} but has recognized inputs",
                                baseline.application_version
                            );
                        }
                    }
                }
            }
        }
        for (type_name, resource_type) in &resolved.resource_types {
            if !selection.types.is_empty() && !selection.types.contains(type_name) {
                continue;
            }
            if resource_type.namespaced
                && !selection.ids.is_empty()
                && selection.namespaces.is_empty()
            {
                bail!(
                    "--namespace is required when exact IDs select namespaced Resource Type {type_name}"
                );
            }
            if !resource_type.namespaced && !selection.namespaces.is_empty() {
                bail!("--namespace is not valid for non-namespaced Resource Type {type_name}");
            }
            let directories = resource_directories(
                &root,
                &project,
                &environment,
                target_name,
                type_name,
                resource_type.namespaced,
            )?;
            for (namespace, directory) in directories {
                if !selection.namespaces.is_empty()
                    && namespace
                        .as_ref()
                        .is_none_or(|value| !selection.namespaces.contains(value))
                {
                    continue;
                }
                let hints = crate::hints::resolve(
                    &root,
                    &project,
                    &environment,
                    target_name,
                    namespace.as_deref(),
                    type_name,
                )?;
                crate::hints::validate_tracking(resource_type, &hints, type_name)?;
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
                    if path.file_name().and_then(|name| name.to_str())
                        == Some(crate::hints::RESOURCE_HINT_NAME)
                    {
                        continue;
                    }
                    let mut value = if let Some(projection) = &resource_type.filesystem {
                        if !metadata.is_dir() {
                            continue;
                        }
                        crate::projection::merge(&path, projection)?
                    } else {
                        if !metadata.is_file() || is_deletion_marker(&path) {
                            continue;
                        }
                        if !matches!(
                            path.extension().and_then(|e| e.to_str()),
                            Some("json" | "json5" | "yaml" | "yml")
                        ) {
                            continue;
                        }
                        parse_resource(&path)?
                    };
                    if !hints.track {
                        value = crate::transport::without_metadata(&value, resource_type)?;
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
                    let extracted_id = canonical_id(&value, resource_type);
                    if pointer_string(&value, TAKU_ID_POINTER).is_none()
                        && let Some(id) = extracted_id.as_deref()
                    {
                        crate::transport::insert_pointer(
                            &mut value,
                            TAKU_ID_POINTER,
                            Value::String(id.to_owned()),
                        )?;
                    }
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
                                TAKU_ID_POINTER
                            )
                        })?
                    };
                    if !selection.ids.is_empty() && !selection.ids.contains(&id) {
                        continue;
                    }
                    let name = display_name_value(&value, &resource_type.display_name)
                        .unwrap_or_else(|| id.clone());
                    let display_name = match resource_type.display_name.strategy {
                        DisplayNameStrategy::Id => id.clone(),
                        DisplayNameStrategy::Name => name.clone(),
                        DisplayNameStrategy::NameId => format!("{}-{}", name, short_id(&id)),
                    };
                    result.push(InventoryEntry {
                        environment: environment.clone(),
                        target: target_name.clone(),
                        namespace: namespace.clone(),
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
    }
    result.sort_by(|a, b| {
        (
            &a.environment,
            &a.target,
            &a.namespace,
            &a.resource_type,
            &a.id,
        )
            .cmp(&(
                &b.environment,
                &b.target,
                &b.namespace,
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
                &item.namespace,
                &item.resource_type,
                &item.display_name,
            ))
        {
            bail!("Display Name collision for {}", item.display_name);
        }
    }
    Ok(result)
}

fn directory_has_recognized_input(
    directory: &Path,
    definitions: &[crate::ResourceType],
) -> Result<bool> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "symlinked Resource input is not allowed: {}",
                path.display()
            );
        }
        if is_deletion_marker(&path)
            || path.file_name().and_then(|name| name.to_str())
                == Some(crate::hints::RESOURCE_HINT_NAME)
            || (metadata.is_dir()
                && definitions
                    .iter()
                    .any(|definition| definition.filesystem.is_some()))
            || (metadata.is_file()
                && definitions
                    .iter()
                    .any(|definition| definition.filesystem.is_none())
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("json" | "json5" | "yaml" | "yml")
                ))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_namespace(namespace: &str) -> Result<()> {
    if namespace.is_empty()
        || matches!(namespace, "." | "..")
        || namespace.contains('/')
        || namespace.contains('\\')
    {
        bail!("Namespace must be one non-empty path segment");
    }
    Ok(())
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

pub(crate) fn target_root(
    root: &Path,
    project: &Project,
    environment: &str,
    target: &str,
) -> PathBuf {
    match project.layout {
        RepositoryLayout::Single => root.join(target),
        RepositoryLayout::Multi => root.join(environment).join(target),
    }
}

pub fn resource_directory_in_namespace(
    root: &Path,
    project: &Project,
    environment: &str,
    target: &str,
    namespace: Option<&str>,
    resource_type: &str,
) -> PathBuf {
    let target_root = target_root(root, project, environment, target);
    match namespace {
        Some(namespace) => target_root.join(namespace).join(resource_type),
        None => target_root.join(resource_type),
    }
}

pub fn resource_directories(
    root: &Path,
    project: &Project,
    environment: &str,
    target: &str,
    resource_type: &str,
    namespaced: bool,
) -> Result<Vec<(Option<String>, PathBuf)>> {
    if !namespaced {
        let directory = resource_directory(root, project, environment, target, resource_type);
        return Ok(directory
            .is_dir()
            .then_some((None, directory))
            .into_iter()
            .collect());
    }

    let target_root = target_root(root, project, environment, target);
    if !target_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut directories = Vec::new();
    for entry in fs::read_dir(target_root)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_dir() {
            continue;
        }
        let namespace = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Namespace directory is not valid UTF-8"))?;
        let directory = entry.path().join(resource_type);
        if directory.is_dir() {
            directories.push((Some(namespace), directory));
        }
    }
    directories.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(directories)
}

pub fn parse_resource(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read Resource {}", path.display()))?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("yaml" | "yml") => serde_yaml::from_str(&text).context("invalid YAML Resource"),
        _ => json5::from_str(&normalize_triple_quotes(&text)).context("invalid JSON Resource"),
    }
}

fn write_resource_with_embedded_yaml(
    path: &Path,
    value: &Value,
    embedded_yaml: &[String],
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = if matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    ) {
        serde_yaml::to_string(value)?
    } else {
        render_json5(value, "", 0, embedded_yaml) + "\n"
    };
    let temporary = path.with_extension("taku.tmp");
    fs::write(&temporary, text)?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub(crate) fn write_canonical_resource(
    path: &Path,
    value: &Value,
    resource_type: &crate::ResourceType,
) -> Result<()> {
    if let Some(projection) = &resource_type.filesystem {
        crate::projection::split(path, projection, value)
    } else {
        let embedded_yaml = resource_type
            .transformations
            .iter()
            .filter_map(|transformation| match transformation {
                crate::Transformation::EmbeddedYaml { pointer } => Some(pointer.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        write_resource_with_embedded_yaml(path, value, &embedded_yaml)
    }
}

/// Render repository JSON as JSON5 when configured fields contain embedded YAML.
/// The YAML remains a string, but multi-line content is emitted with the JSON5
/// triple-quote extension so it can be reviewed and edited without escaped lines.
fn render_json5(value: &Value, pointer: &str, indent: usize, embedded_yaml: &[String]) -> String {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => serde_json::to_string(value).unwrap(),
        Value::String(text)
            if embedded_yaml.iter().any(|candidate| candidate == pointer)
                && text.contains('\n')
                && !text.contains("\"\"\"") =>
        {
            format!("\"\"\"{text}\"\"\"")
        }
        Value::String(_) => serde_json::to_string(value).unwrap(),
        Value::Array(values) if values.is_empty() => "[]".to_owned(),
        Value::Array(values) => {
            let child_indent = indent + 2;
            let padding = " ".repeat(child_indent);
            let closing_padding = " ".repeat(indent);
            let entries = values
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let child_pointer = format!("{pointer}/{index}");
                    format!(
                        "{padding}{}",
                        render_json5(item, &child_pointer, child_indent, embedded_yaml)
                    )
                })
                .collect::<Vec<_>>();
            format!("[\n{}\n{closing_padding}]", entries.join(",\n"))
        }
        Value::Object(values) if values.is_empty() => "{}".to_owned(),
        Value::Object(values) => {
            let child_indent = indent + 2;
            let padding = " ".repeat(child_indent);
            let closing_padding = " ".repeat(indent);
            let entries = values
                .iter()
                .map(|(key, item)| {
                    let escaped_key = key.replace('~', "~0").replace('/', "~1");
                    let child_pointer = format!("{pointer}/{escaped_key}");
                    format!(
                        "{padding}{}: {}",
                        serde_json::to_string(key).unwrap(),
                        render_json5(item, &child_pointer, child_indent, embedded_yaml)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{\n{}\n{closing_padding}}}", entries.join(",\n"))
        }
    }
}

pub(crate) fn remove_canonical_resource(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("symlinked Resource cannot be removed: {}", path.display());
    }
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else if metadata.is_file() {
        fs::remove_file(path)?;
    } else {
        bail!(
            "Resource is not a regular file or directory: {}",
            path.display()
        );
    }
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

fn is_deletion_marker(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".delete.yaml"))
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

#[cfg(test)]
mod tests {
    use super::{
        normalize_triple_quotes, parse_resource, render_json5, write_resource_with_embedded_yaml,
    };
    use serde_json::{Value, json};

    #[test]
    fn embedded_yaml_is_readable_json5_and_round_trips_without_losing_comments() {
        let value = json!({
            "id": "workflow-1",
            "yaml": "# retained comment\nname: User Diagnostic ID Fetcher\nsteps:\n  - name: fetch\n"
        });

        let text = render_json5(&value, "", 0, &["/yaml".to_owned()]);

        assert!(
            text.contains("\"yaml\": \"\"\"# retained comment\nname: User Diagnostic ID Fetcher")
        );
        assert!(!text.contains("\\nname: User Diagnostic ID Fetcher"));
        let parsed: Value = json5::from_str(&normalize_triple_quotes(&text)).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn embedded_yaml_containing_triple_quotes_uses_standard_json_escaping() {
        let value = json!({"yaml": "note: '\"\"\"'\n"});

        let text = render_json5(&value, "", 0, &["/yaml".to_owned()]);

        assert!(!text.contains("\"yaml\": \"\"\""));
        let parsed: Value = json5::from_str(&normalize_triple_quotes(&text)).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn canonical_writer_persists_embedded_yaml_as_a_triple_quoted_json5_value() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workflow.json");
        let value = json!({"yaml": "name: Workflow\nsteps: []\n"});

        write_resource_with_embedded_yaml(&path, &value, &["/yaml".to_owned()]).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"yaml\": \"\"\"name: Workflow\nsteps: []\n\"\"\""));
        assert_eq!(parse_resource(&path).unwrap(), value);
    }
}
