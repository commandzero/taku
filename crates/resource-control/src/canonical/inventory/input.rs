use super::{InventoryTarget, selected};
use crate::canonical::paths::{
    has_resource_extension, is_deletion_marker, reject_symlink_components, resource_paths,
};
use crate::canonical::{
    InventoryEntry, TAKU_ID_POINTER, canonical_id, display_name_value, parse_resource,
    pointer_string, short_id,
};
use crate::{DisplayNameStrategy, IdScope, ResourceType, WriteIntent};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub(super) struct ResourceInput<'a> {
    pub(super) scope: &'a InventoryTarget<'a>,
    pub(super) name: &'a str,
    pub(super) resource_type: &'a ResourceType,
    pub(super) namespace: Option<&'a str>,
}

impl ResourceInput<'_> {
    pub(super) fn collect(&self, directory: &Path, result: &mut Vec<InventoryEntry>) -> Result<()> {
        let scope = self.scope;
        let hints = crate::hints::resolve(
            scope.root,
            scope.project,
            scope.environment,
            scope.name,
            self.namespace,
            self.name,
        )?;
        crate::hints::validate_tracking(self.resource_type, &hints, self.name)?;
        reject_symlink_components(scope.root, directory)?;
        for path in resource_paths(directory, self.resource_type)? {
            result.extend(self.read(&path, hints.track)?);
        }
        Ok(())
    }

    fn read(&self, path: &Path, track: bool) -> Result<Option<InventoryEntry>> {
        let Some(mut value) = read_value(path, self.resource_type)? else {
            return Ok(None);
        };
        if !track {
            value = crate::transport::without_metadata(&value, self.resource_type)?;
        }
        check_filename(path)?;
        self.check_sensitive_fields(path, &value)?;

        let relative = path.strip_prefix(self.scope.root).unwrap();
        let (id, pending) = identify(&mut value, self.resource_type, path, relative)?;
        if !selected(&self.scope.selection.ids, &id) {
            return Ok(None);
        }
        let name = display_name_value(&value, &self.resource_type.display_name)
            .unwrap_or_else(|| id.clone());
        let display_name = match self.resource_type.display_name.strategy {
            DisplayNameStrategy::Id => id.clone(),
            DisplayNameStrategy::Name => name.clone(),
            DisplayNameStrategy::NameId => format!("{}-{}", name, short_id(&id)),
        };
        Ok(Some(InventoryEntry {
            environment: self.scope.environment.to_owned(),
            target: self.scope.name.to_owned(),
            namespace: self.namespace.map(str::to_owned),
            resource_type: self.name.to_owned(),
            id,
            pending,
            name,
            display_name,
            id_scope: self.resource_type.id.scope,
            path: relative.display().to_string(),
            value,
            display_name_unique: self.resource_type.display_name.unique,
        }))
    }

    fn check_sensitive_fields(&self, path: &Path, value: &Value) -> Result<()> {
        let fields = self.resource_type.sensitive_fields.iter().chain(
            self.scope
                .target
                .sensitive_fields
                .get(self.name)
                .into_iter()
                .flatten(),
        );
        for pointer in fields {
            if crate::model::sensitive_field_conflicts(self.resource_type, pointer) {
                bail!(
                    "Sensitive Field {pointer} overlaps required canonical state for {}",
                    self.name
                );
            }
            if value.pointer(pointer).is_some() {
                bail!(
                    "Canonical Resource {} contains Sensitive Field {pointer}",
                    path.display()
                );
            }
        }
        Ok(())
    }
}

fn read_value(path: &Path, resource_type: &ResourceType) -> Result<Option<Value>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "symlinked Resource input is not allowed: {}",
            path.display()
        );
    }
    if path.file_name().and_then(|name| name.to_str()) == Some(crate::hints::RESOURCE_HINT_NAME) {
        return Ok(None);
    }
    match &resource_type.filesystem {
        Some(projection) if metadata.is_dir() => {
            crate::projection::merge(path, projection).map(Some)
        }
        None if metadata.is_file() && is_resource_document(path) => parse_resource(path).map(Some),
        _ => Ok(None),
    }
}

fn is_resource_document(path: &Path) -> bool {
    !is_deletion_marker(path) && has_resource_extension(path)
}

fn check_filename(path: &Path) -> Result<()> {
    let encoded_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Resource filename is not valid UTF-8")?;
    let decoded =
        urlencoding::decode(encoded_name).context("Resource filename contains invalid escapes")?;
    if decoded.contains("..") || decoded.contains(['/', '\\']) {
        bail!("Resource filename contains a traversal escape: {encoded_name}");
    }
    Ok(())
}

fn identify(
    value: &mut Value,
    resource_type: &ResourceType,
    path: &Path,
    relative: &Path,
) -> Result<(String, bool)> {
    if let Some(id) = canonical_id(value, resource_type) {
        if pointer_string(value, TAKU_ID_POINTER).is_none() {
            crate::transport::insert_pointer(value, TAKU_ID_POINTER, Value::String(id.clone()))?;
        }
        return Ok((id, false));
    }
    if resource_type.id.scope == IdScope::Target
        && resource_type.write_intent == WriteIntent::Create
    {
        return Ok((format!("pending:{}", relative.display()), true));
    }
    bail!(
        "Resource {} has no string ID at {TAKU_ID_POINTER}",
        path.display()
    );
}
