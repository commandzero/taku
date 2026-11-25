use super::{INTERNAL_CURSOR_POINTER, INTERNAL_GUARD_POINTER, OperationInput, RemoteResult};
use super::{inbound, insert_pointer, remove_pointer};
use crate::canonical::{TAKU_ID_POINTER, canonical_id, pointer_string, sort_value};
use crate::{Cardinality, Operation, Outcome, Pagination, PayloadFormat, ResourceType};
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::Response;
use serde_json::Value;

pub(super) fn map_response(
    response: Response,
    operation: &Operation,
    resource_type: &ResourceType,
    input: OperationInput<'_>,
) -> Result<RemoteResult> {
    let status = response.status();
    let outcome = operation
        .outcomes
        .get(&status.as_u16())
        .copied()
        .unwrap_or_else(|| conventional_outcome(status));
    match outcome {
        Outcome::Success => map_success(response, operation, resource_type, input),
        Outcome::Failure => Ok(RemoteResult::Failure(format!(
            "remote operation returned HTTP {}",
            status.as_u16()
        ))),
        other => Ok(remote_failure(other)),
    }
}

fn remote_failure(outcome: Outcome) -> RemoteResult {
    match outcome {
        Outcome::NotFound => RemoteResult::NotFound,
        Outcome::Conflict => RemoteResult::Conflict,
        Outcome::Retryable => RemoteResult::Retryable,
        Outcome::Failure | Outcome::Success => unreachable!("expected a classified remote failure"),
    }
}

fn map_success(
    response: Response,
    operation: &Operation,
    resource_type: &ResourceType,
    input: OperationInput<'_>,
) -> Result<RemoteResult> {
    if input.mutation && !operation.consumes_response_body() {
        return Ok(RemoteResult::Success(Vec::new()));
    }
    let bytes = response
        .bytes()
        .map_err(|_| anyhow::anyhow!("failed to read successful remote response"))?;
    if bytes.is_empty() {
        return Ok(RemoteResult::Success(Vec::new()));
    }
    let values = parse_values(&bytes, operation.unbundle)
        .map_err(|_| anyhow::anyhow!("successful remote response could not be parsed safely"))?;
    let next_cursor = match &operation.pagination {
        Some(Pagination::Cursor { next_pointer, .. }) => values
            .first()
            .and_then(|value| pointer_string(value, next_pointer)),
        _ => None,
    };
    let Some(values) = extract_values(values, operation, input.id)? else {
        return handle_missing_extraction(operation);
    };
    let mut normalized = Vec::new();
    for decoded in decode_response(values, operation)? {
        normalized.extend(normalize(decoded, resource_type, operation, input)?);
    }
    if let Some(cursor) = next_cursor {
        let last = normalized
            .last_mut()
            .context("cursor pagination returned no Resources with a continuation")?;
        insert_pointer(last, INTERNAL_CURSOR_POINTER, Value::String(cursor))?;
    }
    Ok(RemoteResult::Success(normalized))
}

fn extract_values(
    values: Vec<Value>,
    operation: &Operation,
    id: Option<&str>,
) -> Result<Option<Vec<Value>>> {
    let Some(extract) = &operation.extract else {
        return Ok(Some(values));
    };
    let value = values.first().context("remote response is empty")?;
    let extracted = if extract == "/{id}" {
        id.and_then(|id| value.get(id))
    } else {
        value.pointer(extract)
    };
    Ok(extracted.cloned().map(|value| vec![value]))
}

fn handle_missing_extraction(operation: &Operation) -> Result<RemoteResult> {
    match operation.extract_missing {
        Some(Outcome::Failure) => Ok(RemoteResult::Failure(
            "configured response extraction did not match".into(),
        )),
        Some(Outcome::Success) | None => bail!("configured response extraction did not match"),
        Some(other) => Ok(remote_failure(other)),
    }
}

fn normalize(
    decoded: DecodedResponse,
    resource_type: &ResourceType,
    operation: &Operation,
    input: OperationInput<'_>,
) -> Result<Option<Value>> {
    let mut value = decoded.value;
    let guard = resource_type
        .guard_pointer
        .as_deref()
        .and_then(|pointer| pointer_string(&value, pointer));
    for pointer in &resource_type.sensitive_fields {
        remove_pointer(&mut value, pointer)?;
    }
    value = inbound(&value, resource_type, input.metadata_track)?;
    let id = requested_identity(operation, input);
    let identity = reconcile_identity(
        &mut value,
        &resource_type.id.pointer,
        decoded.identity.as_deref(),
        id,
    )?;
    if canonical_id(&value, resource_type).is_none() {
        return if operation.skip_unidentified {
            Ok(None)
        } else {
            bail!("remote Resource has no configured ID")
        };
    }
    debug_assert!(identity.is_some());
    if let Some(guard) = guard {
        insert_pointer(&mut value, INTERNAL_GUARD_POINTER, Value::String(guard))?;
    }
    Ok(Some(sort_value(&value)))
}

fn requested_identity<'a>(operation: &Operation, input: OperationInput<'a>) -> Option<&'a str> {
    input
        .id
        .filter(|_| operation.cardinality == Cardinality::One)
}

fn conventional_outcome(status: StatusCode) -> Outcome {
    if status.is_success() {
        Outcome::Success
    } else if status == StatusCode::NOT_FOUND {
        Outcome::NotFound
    } else if status == StatusCode::CONFLICT || status == StatusCode::PRECONDITION_FAILED {
        Outcome::Conflict
    } else if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        Outcome::Retryable
    } else {
        Outcome::Failure
    }
}

fn parse_values(bytes: &[u8], format: Option<PayloadFormat>) -> Result<Vec<Value>> {
    let text = std::str::from_utf8(bytes).context("remote response is not UTF-8")?;
    if format == Some(PayloadFormat::Ndjson) {
        let mut values = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            values.push(json5::from_str(line).context("malformed NDJSON line")?);
        }
        Ok(values)
    } else {
        Ok(vec![
            json5::from_str(text).context("malformed JSON response")?,
        ])
    }
}

pub(super) struct DecodedResponse {
    pub(super) value: Value,
    pub(super) identity: Option<String>,
}

pub(super) fn decode_response(
    values: Vec<Value>,
    operation: &Operation,
) -> Result<Vec<DecodedResponse>> {
    let response = operation.response_mapping();
    response_candidates(values, operation)?
        .into_iter()
        .map(|(map_identity, item)| {
            let pointer_identity = response
                .and_then(|mapping| mapping.identity_pointer.as_deref())
                .map(|pointer| {
                    item.pointer(pointer)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .with_context(|| {
                            format!("response identity pointer {pointer} did not match")
                        })
                })
                .transpose()?;
            let identity = match (map_identity, pointer_identity) {
                (Some(map), Some(pointer)) if map != pointer => {
                    bail!("response identities disagree: {map} != {pointer}")
                }
                (Some(identity), _) | (_, Some(identity)) => Some(identity),
                (None, None) => None,
            };
            let value = if let Some(pointer) =
                response.and_then(|mapping| mapping.resource_pointer.as_deref())
            {
                item.pointer(pointer)
                    .cloned()
                    .with_context(|| format!("response Resource pointer {pointer} did not match"))?
            } else {
                item
            };
            Ok(DecodedResponse { value, identity })
        })
        .collect()
}

fn response_candidates(
    values: Vec<Value>,
    operation: &Operation,
) -> Result<Vec<(Option<String>, Value)>> {
    let candidates = match operation
        .response_mapping()
        .and_then(|mapping| mapping.collection)
    {
        Some(crate::CollectionShape::List) => {
            if operation.unbundle == Some(crate::PayloadFormat::Ndjson) {
                values.into_iter().map(|value| (None, value)).collect()
            } else {
                let [value]: [Value; 1] = values
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("list response must contain one JSON value"))?;
                value
                    .as_array()
                    .context("configured list response is not an array")?
                    .iter()
                    .cloned()
                    .map(|value| (None, value))
                    .collect()
            }
        }
        Some(crate::CollectionShape::Map) => {
            let [value]: [Value; 1] = values
                .try_into()
                .map_err(|_| anyhow::anyhow!("map response must contain one JSON value"))?;
            let map = value
                .as_object()
                .context("configured map response is not an object")?;
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(id, _)| *id);
            entries
                .into_iter()
                .map(|(id, value)| (Some(id.clone()), value.clone()))
                .collect()
        }
        None => values.into_iter().map(|value| (None, value)).collect(),
    };
    Ok(candidates)
}

pub(super) fn reconcile_identity(
    value: &mut Value,
    pointer: &str,
    decoded: Option<&str>,
    requested: Option<&str>,
) -> Result<Option<String>> {
    if let (Some(decoded), Some(requested)) = (decoded, requested)
        && decoded != requested
    {
        bail!("decoded Resource identity {decoded} disagrees with requested identity {requested}");
    }
    let wire_identity = pointer_string(value, pointer);
    let canonical_identity = pointer_string(value, TAKU_ID_POINTER);
    let identity = decoded
        .or(requested)
        .or(canonical_identity.as_deref())
        .or(wire_identity.as_deref());
    if let (Some(existing), Some(identity)) = (canonical_identity.as_deref(), identity)
        && existing != identity
    {
        bail!("decoded Resource identity {identity} disagrees with canonical identity {existing}");
    }
    if let Some(identity) = identity {
        if canonical_identity.as_deref() != Some(identity) {
            insert_pointer(value, TAKU_ID_POINTER, Value::String(identity.into()))?;
        }
        Ok(Some(identity.to_owned()))
    } else {
        Ok(None)
    }
}
