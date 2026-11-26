use super::pointers::{insert_pointer, remove_pointer, remove_pointers};
use crate::canonical::{TAKU_NAMESPACE_POINTER, canonical_id, pointer_string, sort_value};
use crate::{Operation, ResourceType};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

fn apply_inbound(value: &mut Value, resource_type: &ResourceType) -> Result<()> {
    for transformation in &resource_type.transformations {
        match transformation {
            crate::Transformation::Extract { pointer } => {
                *value = value
                    .pointer(pointer)
                    .cloned()
                    .context("Transformation extraction did not match")?;
            }
            crate::Transformation::Remove { pointer } => remove_pointer(value, pointer)?,
            crate::Transformation::Omit { .. } => {}
            crate::Transformation::Insert {
                pointer,
                value: inserted,
            } => insert_pointer(value, pointer, inserted.clone())?,
            crate::Transformation::EmbeddedJson { pointer } => {
                if let Some(text) = value.pointer(pointer).and_then(Value::as_str) {
                    let parsed: Value =
                        json5::from_str(text).context("embedded JSON is malformed")?;
                    insert_pointer(value, pointer, sort_value(&parsed))?;
                }
            }
            // YAML is kept as source text so comments and user formatting survive a
            // fetch/push round trip. Canonical storage renders it readably.
            crate::Transformation::EmbeddedYaml { .. } => {}
            crate::Transformation::SingletonMap {
                pointer,
                key_pointer,
                value_pointer,
            } => {
                let singleton = value
                    .pointer(pointer)
                    .and_then(Value::as_object)
                    .context("Singleton Map input is not an object")?;
                if singleton.len() != 1 {
                    bail!("Singleton Map input must contain exactly one entry");
                }
                let (key, document) = singleton.iter().next().unwrap();
                let key = key.clone();
                let document = document.clone();
                remove_pointer(value, pointer)?;
                insert_pointer(value, key_pointer, Value::String(key))?;
                insert_pointer(value, value_pointer, document)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn inbound(
    value: &Value,
    resource_type: &ResourceType,
    metadata_track: bool,
) -> Result<Value> {
    let mut value = value.clone();
    apply_inbound(&mut value, resource_type)?;
    if !metadata_track && let Some(metadata) = &resource_type.metadata {
        remove_pointers(&mut value, &metadata.fields)?;
    }
    Ok(value)
}

pub fn outbound(
    value: &Value,
    resource_type: &ResourceType,
    operation: Option<&Operation>,
) -> Result<Value> {
    let mut value = without_metadata(value, resource_type)?;
    if let Some(operation) = operation {
        set_wire_identity(&mut value, resource_type, operation)?;
    }
    // Taku-managed canonical state is repository metadata, never API payload.
    // Remove the whole reserved namespace so future fields are safe by default.
    remove_pointer(&mut value, TAKU_NAMESPACE_POINTER)?;
    apply_outbound(&mut value, &resource_type.transformations)?;
    if let Some(operation) = operation {
        apply_outbound(&mut value, &operation.transformations)?;
        if let Some(crate::OperationBody::Pointer(pointer)) = &operation.body {
            return value
                .pointer(pointer)
                .cloned()
                .with_context(|| format!("Operation body selector {pointer} did not match"));
        }
    }
    Ok(value)
}

fn set_wire_identity(
    value: &mut Value,
    resource_type: &ResourceType,
    operation: &Operation,
) -> Result<()> {
    let pointer = &resource_type.id.pointer;
    let identity = canonical_id(value, resource_type);
    let wire_identity = pointer_string(value, pointer);
    if operation.includes_identity_in_body() {
        if let (None, Some(identity)) = (wire_identity, identity) {
            insert_pointer(value, pointer, Value::String(identity))?;
        }
    } else if wire_identity.is_none() || wire_identity == identity {
        remove_pointer(value, pointer)?;
    }
    Ok(())
}

pub(crate) fn without_metadata(value: &Value, resource_type: &ResourceType) -> Result<Value> {
    let mut value = value.clone();
    if let Some(metadata) = &resource_type.metadata {
        remove_pointers(&mut value, &metadata.fields)?;
    }
    Ok(value)
}

fn apply_outbound(value: &mut Value, transformations: &[crate::Transformation]) -> Result<()> {
    for transformation in transformations.iter().rev() {
        match transformation {
            crate::Transformation::EmbeddedJson { pointer } => {
                if let Some(document) = value
                    .pointer(pointer)
                    .filter(|value| value.is_object() || value.is_array())
                    .cloned()
                {
                    insert_pointer(
                        value,
                        pointer,
                        Value::String(serde_json::to_string(&document)?),
                    )?;
                }
            }
            crate::Transformation::EmbeddedYaml { .. } => {}
            crate::Transformation::Extract { pointer } => {
                let document = value.clone();
                *value = Value::Object(Map::new());
                insert_pointer(value, pointer, document)?;
            }
            crate::Transformation::Insert { pointer, .. } => remove_pointer(value, pointer)?,
            crate::Transformation::Omit { pointer } => remove_pointer(value, pointer)?,
            crate::Transformation::Remove { .. } => {}
            crate::Transformation::SingletonMap {
                pointer,
                key_pointer,
                value_pointer,
            } => {
                let key = pointer_string(value, key_pointer)
                    .context("Singleton Map key is not a string")?;
                let document = value
                    .pointer(value_pointer)
                    .cloned()
                    .context("Singleton Map value is missing")?;
                remove_pointer(value, key_pointer)?;
                remove_pointer(value, value_pointer)?;
                let mut singleton = Map::new();
                singleton.insert(key, document);
                insert_pointer(value, pointer, Value::Object(singleton))?;
            }
        }
    }
    Ok(())
}
