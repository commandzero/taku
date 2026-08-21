use crate::canonical::{
    TAKU_ID_POINTER, TAKU_NAMESPACE_POINTER, canonical_id, pointer_string, sort_value,
};
use crate::{
    ApplicationDefinition, Operation, Outcome, PayloadFormat, ResourceType, TargetConfig,
    VersionEndpoint,
};
use anyhow::{Context, Result, bail};
use redact::Secret;
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::Duration;

pub enum RemoteResult {
    Success(Vec<Value>),
    NotFound,
    Conflict,
    Retryable,
    Failure(String),
    Uncertain,
}

#[derive(Clone, Copy, Default)]
pub struct OperationInput<'a> {
    pub namespace: Option<&'a str>,
    pub id: Option<&'a str>,
    pub context: Option<&'a Value>,
    pub body: Option<&'a [Value]>,
    pub resource_ids: Option<&'a [String]>,
    pub mutation: bool,
    pub metadata_track: bool,
}
pub const INTERNAL_GUARD_POINTER: &str = "/_taku/guard";
pub const INTERNAL_CURSOR_POINTER: &str = "/_taku/cursor";

pub fn execute_version_endpoint(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    endpoint: &VersionEndpoint,
    auth: &BTreeMap<String, Secret<String>>,
) -> Result<Value> {
    let base_url = auth
        .get("url")
        .map(|value| value.expose_secret().as_str())
        .unwrap_or(&target.url);
    let path = render_path(&endpoint.path, None, None, None);
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let method =
        Method::from_bytes(endpoint.method.as_bytes()).context("invalid configured HTTP method")?;
    let mut request = client.request(method, &url);
    for (name, value) in &app.target_profile.headers {
        request = request.header(name, value);
    }
    for (name, value) in &target.headers {
        request = request.header(name, value);
    }
    for (name, value) in &endpoint.headers {
        request = request.header(name, value);
    }
    for (name, value) in auth.iter().filter(|(name, _)| name.as_str() != "url") {
        request = request.header(name, value.expose_secret());
    }
    let response = request
        .send()
        .map_err(|_| anyhow::anyhow!("Version Endpoint request failed"))?;
    if !response.status().is_success() {
        bail!(
            "Version Endpoint returned HTTP {}",
            response.status().as_u16()
        );
    }
    let bytes = response
        .bytes()
        .map_err(|_| anyhow::anyhow!("Version Endpoint response failed"))?;
    json5::from_str(
        std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("Version Endpoint response is not UTF-8"))?,
    )
    .map_err(|_| anyhow::anyhow!("Version Endpoint response is invalid"))
}

pub fn execute(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    resource_type: &ResourceType,
    operation: &Operation,
    input: OperationInput<'_>,
    auth: &BTreeMap<String, Secret<String>>,
) -> Result<RemoteResult> {
    let operation_path = operation_path(operation, input.namespace);
    let path = render_path(
        &operation_path,
        input.namespace,
        input.id,
        input
            .context
            .or_else(|| input.body.and_then(|values| values.first())),
    );
    let request_body = build_request_body(
        operation,
        input.namespace,
        input.id,
        input.context,
        input.body,
        input.resource_ids,
    )?;
    let base_url = auth
        .get("url")
        .map(|value| value.expose_secret().as_str())
        .unwrap_or(&target.url);
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let method = Method::from_bytes(operation.method.as_bytes())
        .context("invalid configured HTTP method")?;
    let mut request = client.request(method, &url);
    if !operation.query.is_empty() {
        request = request.query(&operation.query);
    }
    for (name, value) in &app.target_profile.headers {
        request = request.header(name, value);
    }
    for (name, value) in &target.headers {
        request = request.header(name, value);
    }
    for (name, value) in &operation.headers {
        request = request.header(name, value);
    }
    for (name, value) in auth.iter().filter(|(name, _)| name.as_str() != "url") {
        request = request.header(name, value.expose_secret());
    }
    if let Some(values) = request_body.as_deref() {
        let format = operation.bundle.as_ref().map(|bundle| bundle.format);
        let body = encode_payload(values, format)?;
        request = if let Some(multipart) = operation
            .bundle
            .as_ref()
            .and_then(|bundle| bundle.multipart.as_ref())
        {
            let part = reqwest::blocking::multipart::Part::bytes(body)
                .file_name(multipart.filename.clone())
                .mime_str(&multipart.content_type)?;
            request.multipart(
                reqwest::blocking::multipart::Form::new().part(multipart.name.clone(), part),
            )
        } else if format == Some(PayloadFormat::Ndjson) {
            request
                .body(body)
                .header("content-type", "application/x-ndjson")
        } else if values.len() == 1 {
            request.json(&values[0])
        } else {
            request.json(values)
        };
    }
    let response = match request.send() {
        Ok(response) => response,
        Err(error) if error.is_timeout() || error.is_connect() => {
            return Ok(RemoteResult::Uncertain);
        }
        Err(_) => return Ok(RemoteResult::Failure("remote request failed".into())),
    };
    map_response(
        response,
        operation,
        resource_type,
        input.id,
        !input.mutation || operation.consumes_response_body(),
        input.metadata_track,
    )
    .context("Transformation Conflict while converting successful response")
}

fn encode_payload(values: &[Value], format: Option<PayloadFormat>) -> Result<Vec<u8>> {
    if format == Some(PayloadFormat::Ndjson) {
        let mut framed = Vec::new();
        for value in values {
            serde_json::to_writer(&mut framed, value)?;
            framed.push(b'\n');
        }
        Ok(framed)
    } else if values.len() == 1 {
        Ok(serde_json::to_vec(&values[0])?)
    } else {
        Ok(serde_json::to_vec(values)?)
    }
}

pub fn execute_retry_safe(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    resource_type: &ResourceType,
    operation: &Operation,
    input: OperationInput<'_>,
    auth: &BTreeMap<String, Secret<String>>,
) -> Result<RemoteResult> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let result = execute(target, app, resource_type, operation, input, auth)?;
        if matches!(result, RemoteResult::Retryable | RemoteResult::Uncertain)
            && operation.retry_safe
            && attempts < 3
        {
            continue;
        }
        return Ok(result);
    }
}

fn build_request_body(
    operation: &Operation,
    namespace: Option<&str>,
    id: Option<&str>,
    context: Option<&Value>,
    resources: Option<&[Value]>,
    resource_ids: Option<&[String]>,
) -> Result<Option<Vec<Value>>> {
    let shaped_resources = resources
        .map(|resources| shape_request_resources(operation, resources, resource_ids))
        .transpose()?;
    let Some(crate::OperationBody::Template(template)) = operation.body.as_ref() else {
        return Ok(shaped_resources);
    };
    let mut body = render_body_template(
        template,
        namespace,
        id,
        context.or_else(|| resources.and_then(|values| values.first())),
    );
    if let Some(resources) = shaped_resources.as_deref()
        && let Some(pointer) = operation.body_pointer.as_deref()
    {
        let inserted =
            if operation.cardinality == crate::Cardinality::Many && operation.bundle.is_none() {
                Value::Array(resources.to_vec())
            } else {
                resources
                    .first()
                    .cloned()
                    .context("One Operation has no Resource payload")?
            };
        insert_pointer(&mut body, pointer, inserted)?;
    }
    Ok(Some(vec![body]))
}

fn shape_request_resources(
    operation: &Operation,
    resources: &[Value],
    resource_ids: Option<&[String]>,
) -> Result<Vec<Value>> {
    match operation.bundle.as_ref() {
        Some(bundle) if bundle.shape == crate::CollectionShape::Map => {
            let ids = resource_ids.context("map bundle requires Resource IDs")?;
            if ids.len() != resources.len() {
                bail!("map bundle Resource IDs do not match its values");
            }
            let mut map = Map::new();
            for (id, value) in ids.iter().zip(resources) {
                if map.insert(id.clone(), value.clone()).is_some() {
                    bail!("map bundle contains duplicate Resource ID {id}");
                }
            }
            Ok(vec![Value::Object(map)])
        }
        Some(bundle)
            if bundle.shape == crate::CollectionShape::List
                && bundle.format == PayloadFormat::Json =>
        {
            Ok(vec![Value::Array(resources.to_vec())])
        }
        _ => Ok(resources.to_vec()),
    }
}

fn render_body_template(
    template: &Value,
    namespace: Option<&str>,
    id: Option<&str>,
    resource: Option<&Value>,
) -> Value {
    match template {
        Value::String(value) => {
            let mut rendered = value.replace("{id}", id.unwrap_or(""));
            rendered = rendered.replace("{namespace}", namespace.unwrap_or(""));
            if let Some(fields) = resource.and_then(Value::as_object) {
                for (name, value) in fields {
                    if let Some(value) = value.as_str() {
                        rendered = rendered.replace(&format!("{{{name}}}"), value);
                    }
                }
            }
            Value::String(rendered)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| render_body_template(value, namespace, id, resource))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        render_body_template(value, namespace, id, resource),
                    )
                })
                .collect(),
        ),
        value => value.clone(),
    }
}

fn map_response(
    response: Response,
    operation: &Operation,
    resource_type: &ResourceType,
    id: Option<&str>,
    normalize_success: bool,
    metadata_track: bool,
) -> Result<RemoteResult> {
    let status = response.status();
    let outcome = operation
        .outcomes
        .get(&status.as_u16())
        .copied()
        .unwrap_or_else(|| conventional_outcome(status));
    match outcome {
        Outcome::NotFound => Ok(RemoteResult::NotFound),
        Outcome::Conflict => Ok(RemoteResult::Conflict),
        Outcome::Retryable => Ok(RemoteResult::Retryable),
        Outcome::Failure => Ok(RemoteResult::Failure(format!(
            "remote operation returned HTTP {}",
            status.as_u16()
        ))),
        Outcome::Success => {
            if !normalize_success {
                return Ok(RemoteResult::Success(Vec::new()));
            }
            let bytes = response
                .bytes()
                .map_err(|_| anyhow::anyhow!("failed to read successful remote response"))?;
            if bytes.is_empty() {
                return Ok(RemoteResult::Success(Vec::new()));
            }
            let mut values = parse_values(&bytes, operation.unbundle).map_err(|_| {
                anyhow::anyhow!("successful remote response could not be parsed safely")
            })?;
            let next_cursor = match &operation.pagination {
                Some(crate::Pagination::Cursor { next_pointer, .. }) => values
                    .first()
                    .and_then(|value| pointer_string(value, next_pointer)),
                _ => None,
            };
            if let Some(extract) = &operation.extract {
                let value = values
                    .into_iter()
                    .next()
                    .context("remote response is empty")?;
                let extracted = if extract == "/{id}" {
                    id.and_then(|id| value.get(id)).cloned()
                } else {
                    value.pointer(extract).cloned()
                };
                let Some(extracted) = extracted else {
                    return match operation.extract_missing {
                        Some(Outcome::NotFound) => Ok(RemoteResult::NotFound),
                        Some(Outcome::Conflict) => Ok(RemoteResult::Conflict),
                        Some(Outcome::Retryable) => Ok(RemoteResult::Retryable),
                        Some(Outcome::Failure) => Ok(RemoteResult::Failure(
                            "configured response extraction did not match".into(),
                        )),
                        Some(Outcome::Success) | None => {
                            bail!("configured response extraction did not match")
                        }
                    };
                };
                values = vec![extracted];
            }
            let decoded_values = decode_response(values, operation)?;
            let mut normalized = Vec::with_capacity(decoded_values.len());
            for decoded in decoded_values {
                let mut value = decoded.value;
                let guard = resource_type
                    .guard_pointer
                    .as_deref()
                    .and_then(|pointer| pointer_string(&value, pointer));
                for pointer in &resource_type.sensitive_fields {
                    remove_pointer(&mut value, pointer)?;
                }
                value = inbound(&value, resource_type, metadata_track)?;
                let identity = reconcile_identity(
                    &mut value,
                    &resource_type.id.pointer,
                    decoded.identity.as_deref(),
                    (operation.cardinality == crate::Cardinality::One)
                        .then_some(id)
                        .flatten(),
                )?;
                if canonical_id(&value, resource_type).is_none() {
                    if operation.skip_unidentified {
                        continue;
                    }
                    bail!("remote Resource has no configured ID");
                }
                debug_assert!(identity.is_some());
                if let Some(guard) = guard {
                    insert_pointer(&mut value, INTERNAL_GUARD_POINTER, Value::String(guard))?;
                }
                normalized.push(sort_value(&value));
            }
            let mut values = normalized;
            if let Some(cursor) = next_cursor {
                let last = values
                    .last_mut()
                    .context("cursor pagination returned no Resources with a continuation")?;
                insert_pointer(last, INTERNAL_CURSOR_POINTER, Value::String(cursor))?;
            }
            Ok(RemoteResult::Success(values))
        }
    }
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

struct DecodedResponse {
    value: Value,
    identity: Option<String>,
}

fn decode_response(values: Vec<Value>, operation: &Operation) -> Result<Vec<DecodedResponse>> {
    let response = operation.response_mapping();
    let candidates: Vec<(Option<String>, Value)> = match response.and_then(|r| r.collection) {
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
    candidates
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

fn reconcile_identity(
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
        let identity = canonical_id(&value, resource_type);
        if operation.includes_identity_in_body()
            && pointer_string(&value, &resource_type.id.pointer).is_none()
            && let Some(identity) = identity.as_deref()
        {
            insert_pointer(
                &mut value,
                &resource_type.id.pointer,
                Value::String(identity.to_owned()),
            )?;
        }
        if !operation.includes_identity_in_body() {
            let wire_identity = pointer_string(&value, &resource_type.id.pointer);
            if wire_identity.is_none() || wire_identity == identity {
                remove_pointer(&mut value, &resource_type.id.pointer)?;
            }
        }
    }
    // Taku-managed canonical state is repository metadata, never API payload.
    // Remove the whole reserved namespace so future fields are safe by default.
    remove_pointer(&mut value, TAKU_NAMESPACE_POINTER)?;
    apply_outbound(&mut value, &resource_type.transformations)?;
    if let Some(operation) = operation {
        apply_outbound(&mut value, &operation.transformations)?;
        if let Some(crate::OperationBody::Pointer(pointer)) = &operation.body {
            value = value
                .pointer(pointer)
                .cloned()
                .with_context(|| format!("Operation body selector {pointer} did not match"))?;
        }
    }
    Ok(value)
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

fn operation_path<'a>(operation: &'a Operation, namespace: Option<&str>) -> Cow<'a, str> {
    if namespace == Some("default") {
        Cow::Borrowed(&operation.path)
    } else if namespace.is_some()
        && let Some(wrapper) = &operation.namespace
    {
        Cow::Owned(format!(
            "{}{}{}",
            wrapper.prefix.as_deref().unwrap_or_default(),
            operation.path,
            wrapper.suffix.as_deref().unwrap_or_default()
        ))
    } else {
        Cow::Borrowed(&operation.path)
    }
}

fn render_path(
    template: &str,
    namespace: Option<&str>,
    id: Option<&str>,
    value: Option<&Value>,
) -> String {
    let mut path = template.replace("{id}", &urlencoding::encode(id.unwrap_or("")));
    path = path.replace("{namespace}", &urlencoding::encode(namespace.unwrap_or("")));
    if let Some(fields) = value.and_then(Value::as_object) {
        for (name, value) in fields {
            if let Some(value) = value.as_str() {
                path = path.replace(&format!("{{{name}}}"), &urlencoding::encode(value));
            }
        }
    }
    path
}

pub fn insert_pointer(root: &mut Value, pointer: &str, inserted: Value) -> Result<()> {
    let tokens: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    if tokens.is_empty() {
        *root = inserted;
        return Ok(());
    }
    let mut current = root;
    for token in &tokens[..tokens.len() - 1] {
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        current = current
            .as_object_mut()
            .unwrap()
            .entry(token)
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if !current.is_object() {
        bail!("Transformation pointer parent is not an object");
    }
    current
        .as_object_mut()
        .unwrap()
        .insert(tokens.last().unwrap().clone(), inserted);
    Ok(())
}

pub fn remove_pointer(root: &mut Value, pointer: &str) -> Result<()> {
    let mut tokens: Vec<&str> = pointer.split('/').skip(1).collect();
    let Some(last) = tokens.pop() else {
        bail!("cannot remove document root");
    };
    let mut current = root;
    for raw in tokens {
        let token = raw.replace("~1", "/").replace("~0", "~");
        let next = match current {
            Value::Object(map) => map.get_mut(&token),
            Value::Array(values) => token
                .parse::<usize>()
                .ok()
                .and_then(|index| values.get_mut(index)),
            _ => None,
        };
        let Some(next) = next else {
            return Ok(());
        };
        current = next;
    }
    let last = last.replace("~1", "/").replace("~0", "~");
    match current {
        Value::Object(map) => {
            map.remove(&last);
        }
        Value::Array(values) => {
            if let Ok(index) = last.parse::<usize>()
                && index < values.len()
            {
                values.remove(index);
            }
        }
        _ => {}
    }
    Ok(())
}

fn remove_pointers(root: &mut Value, pointers: &[String]) -> Result<()> {
    let mut pointers = pointers.iter().map(String::as_str).collect::<Vec<_>>();
    pointers.sort_by(|left, right| {
        let left = left.split('/').skip(1);
        let right = right.split('/').skip(1);
        for (left, right) in left.zip(right) {
            if left == right {
                continue;
            }
            return match (left.parse::<usize>(), right.parse::<usize>()) {
                (Ok(left), Ok(right)) => right.cmp(&left),
                _ => left.cmp(right),
            };
        }
        std::cmp::Ordering::Equal
    });
    for pointer in pointers {
        remove_pointer(root, pointer)?;
    }
    Ok(())
}

#[cfg(test)]
mod namespace_path_tests {
    use super::{
        build_request_body, decode_response, encode_payload, inbound, operation_path, outbound,
        reconcile_identity, render_path, shape_request_resources,
    };
    use crate::{Operation, ResourceType};

    #[test]
    fn namespace_suffix_wraps_only_named_namespaces() {
        let operation: Operation = serde_yaml::from_str(
            r#"
method: GET
path: /api/widgets
namespace: { suffix: "/namespaces/{namespace}" }
cardinality: many
"#,
        )
        .unwrap();

        let named = operation_path(&operation, Some("blue team"));
        assert_eq!(
            render_path(&named, Some("blue team"), None, None),
            "/api/widgets/namespaces/blue%20team"
        );
        assert_eq!(operation_path(&operation, Some("default")), "/api/widgets");
        assert_eq!(operation_path(&operation, None), "/api/widgets");
    }

    fn metadata_type() -> ResourceType {
        serde_yaml::from_str(
            r#"
id: { pointer: /id, scope: universal }
display_name: { strategy: id }
metadata: { fields: [/audit/created_by, /updated_at] }
transformations: [{ kind: extract, pointer: /document }]
operations: {}
"#,
        )
        .unwrap()
    }

    #[test]
    fn inbound_metadata_is_removed_only_when_directory_tracking_is_disabled() {
        let response = serde_json::json!({
            "document": {"id": "one", "name": "One", "audit": {"created_by": "sam"}}
        });
        assert_eq!(
            inbound(&response, &metadata_type(), false).unwrap(),
            serde_json::json!({"id": "one", "name": "One", "audit": {}})
        );
        assert_eq!(
            inbound(&response, &metadata_type(), true).unwrap(),
            serde_json::json!({"id": "one", "name": "One", "audit": {"created_by": "sam"}})
        );
    }

    #[test]
    fn outbound_metadata_is_removed_before_resource_and_operation_conversion() {
        let operation: Operation = serde_yaml::from_str(
            r#"
method: PUT
path: /items
transformations: [{ kind: extract, pointer: /payload }]
"#,
        )
        .unwrap();
        let value = serde_json::json!({
            "id": "one", "name": "One", "audit": {"created_by": "sam"}, "updated_at": "today"
        });

        assert_eq!(
            outbound(&value, &metadata_type(), Some(&operation)).unwrap(),
            serde_json::json!({"payload": {"document": {"id": "one", "name": "One", "audit": {}}}})
        );
    }

    #[test]
    fn outbound_metadata_removes_array_elements() {
        let resource_type: ResourceType = serde_yaml::from_str(
            r#"
id: { pointer: /id, scope: universal }
display_name: { strategy: id }
metadata: { fields: [/audit/0, /audit/1] }
operations: {}
"#,
        )
        .unwrap();
        let value = serde_json::json!({"id": "one", "audit": ["secret-a", "secret-b", "kept"]});

        assert_eq!(
            outbound(&value, &resource_type, None).unwrap(),
            serde_json::json!({"id": "one", "audit": ["kept"]})
        );
    }

    #[test]
    fn response_decoding_supports_direct_list_and_keyed_map_shapes() {
        let direct: Operation = serde_yaml::from_str("method: GET\npath: /items/{id}\n").unwrap();
        let decoded = decode_response(vec![serde_json::json!({"value": 1})], &direct).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].value, serde_json::json!({"value": 1}));
        assert_eq!(decoded[0].identity, None);

        let list: Operation = serde_yaml::from_str(
            r#"
method: GET
path: /items
cardinality: many
response:
  collection: list
  identity_pointer: /name
  resource_pointer: /item
"#,
        )
        .unwrap();
        let decoded = decode_response(
            vec![serde_json::json!([
                {"name": "two", "item": {"value": 2}},
                {"name": "one", "item": {"value": 1}}
            ])],
            &list,
        )
        .unwrap();
        assert_eq!(decoded[0].identity.as_deref(), Some("two"));
        assert_eq!(decoded[0].value, serde_json::json!({"value": 2}));

        let map: Operation = serde_yaml::from_str(
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: map}\n",
        )
        .unwrap();
        let decoded = decode_response(
            vec![serde_json::json!({
                "two": {"version": 2},
                "one": {"version": 1}
            })],
            &map,
        )
        .unwrap();
        assert_eq!(decoded[0].identity.as_deref(), Some("one"));
        assert_eq!(decoded[0].value, serde_json::json!({"version": 1}));
        assert_eq!(decoded[1].identity.as_deref(), Some("two"));
    }

    #[test]
    fn response_decoding_rejects_shape_pointer_and_identity_conflicts() {
        let list: Operation = serde_yaml::from_str(
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: list}\n",
        )
        .unwrap();
        assert!(decode_response(vec![serde_json::json!({})], &list).is_err());

        let map: Operation = serde_yaml::from_str(
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: map, resource_pointer: /item}\n",
        )
        .unwrap();
        assert!(decode_response(vec![serde_json::json!({"one": {}})], &map).is_err());

        let pointer: Operation = serde_yaml::from_str(
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: list, identity_pointer: /id}\n",
        )
        .unwrap();
        assert!(decode_response(vec![serde_json::json!([{"id": 42}])], &pointer).is_err());

        let mut value = serde_json::json!({"_taku": {"id": "inside"}});
        assert!(reconcile_identity(&mut value, "/id", Some("outside"), None).is_err());
        assert!(reconcile_identity(&mut value, "/id", Some("inside"), Some("requested")).is_err());
    }

    #[test]
    fn requested_identity_is_persisted_in_canonical_resource() {
        let mut value = serde_json::json!({"version": 1, "policy": {"phases": {}}});
        let identity = reconcile_identity(&mut value, "/id", None, Some("logs")).unwrap();
        assert_eq!(identity.as_deref(), Some("logs"));
        assert_eq!(value["_taku"]["id"], "logs");
    }

    #[test]
    fn requested_identity_does_not_overwrite_wire_fields() {
        let mut value = serde_json::json!({"name": "display-name"});
        reconcile_identity(&mut value, "/name", None, Some("resource-id")).unwrap();
        assert_eq!(value["name"], "display-name");
        assert_eq!(value["_taku"]["id"], "resource-id");

        let resource_type: ResourceType = serde_yaml::from_str(
            "id: { pointer: /name, scope: universal }\ndisplay_name: { strategy: name }\noperations: {}\n",
        )
        .unwrap();
        let operation: Operation =
            serde_yaml::from_str("method: PUT\npath: /items/{id}\n").unwrap();
        let canonical = serde_json::json!({
            "_taku": {"id": "resource-id"},
            "name": "display-name",
            "value": 1
        });
        assert_eq!(
            outbound(&canonical, &resource_type, Some(&operation)).unwrap(),
            serde_json::json!({"name": "display-name", "value": 1})
        );
    }

    #[test]
    fn path_identity_metadata_and_body_wrapper_are_omitted_from_mutation() {
        let resource_type: ResourceType = serde_yaml::from_str(
            r#"
id: { pointer: /id, scope: universal }
display_name: { strategy: id }
metadata: { fields: [/version, /modified_date] }
operations: {}
"#,
        )
        .unwrap();
        let operation: Operation =
            serde_yaml::from_str("method: PUT\npath: /policies/{id}\nbody: /policy\n").unwrap();
        let canonical = serde_json::json!({
            "id": "logs",
            "version": 1,
            "modified_date": "2026-01-01",
            "policy": {"phases": {"hot": {}}}
        });
        assert_eq!(
            outbound(&canonical, &resource_type, Some(&operation)).unwrap(),
            serde_json::json!({"phases": {"hot": {}}})
        );
    }

    #[test]
    fn identity_body_defaults_and_overrides_are_honored() {
        let resource_type: ResourceType = serde_yaml::from_str(
            r#"
id: { pointer: /identity/id, scope: universal }
display_name: { strategy: id }
operations: {}
"#,
        )
        .unwrap();
        let value = serde_json::json!({"identity": {"id": "one"}, "value": 1});

        let path_default: Operation =
            serde_yaml::from_str("method: PUT\npath: /items/{id}\n").unwrap();
        assert_eq!(
            outbound(&value, &resource_type, Some(&path_default)).unwrap(),
            serde_json::json!({"identity": {}, "value": 1})
        );

        let path_override: Operation =
            serde_yaml::from_str("method: PUT\npath: /items/{id}\nidentity_in_body: true\n")
                .unwrap();
        assert_eq!(
            outbound(&value, &resource_type, Some(&path_override)).unwrap(),
            value
        );

        let canonical_only = serde_json::json!({
            "_taku": {"id": "one"},
            "value": 1
        });
        assert_eq!(
            outbound(&canonical_only, &resource_type, Some(&path_override)).unwrap(),
            serde_json::json!({"identity": {"id": "one"}, "value": 1})
        );

        let body_override: Operation =
            serde_yaml::from_str("method: POST\npath: /items\nidentity_in_body: false\n").unwrap();
        assert_eq!(
            outbound(&value, &resource_type, Some(&body_override)).unwrap(),
            serde_json::json!({"identity": {}, "value": 1})
        );

        let future_taku_state = serde_json::json!({
            "_taku": {"id": "one", "provenance": {"source": "remote"}},
            "value": 1
        });
        assert_eq!(
            outbound(&future_taku_state, &resource_type, Some(&body_override)).unwrap(),
            serde_json::json!({"value": 1})
        );
    }

    #[test]
    fn list_and_map_request_bundles_are_explicit_and_deterministic() {
        let values = vec![
            serde_json::json!({"value": 2}),
            serde_json::json!({"value": 1}),
        ];
        let ids = vec!["two".to_owned(), "one".to_owned()];
        let list: Operation = serde_yaml::from_str(
            "method: POST\npath: /items\ncardinality: many\nbundle: {shape: list, format: json}\n",
        )
        .unwrap();
        assert_eq!(
            shape_request_resources(&list, &values, Some(&ids)).unwrap(),
            vec![serde_json::json!([{"value": 2}, {"value": 1}])]
        );
        assert_eq!(
            shape_request_resources(&list, &values[..1], Some(&ids[..1])).unwrap(),
            vec![serde_json::json!([{"value": 2}])]
        );

        let map: Operation = serde_yaml::from_str(
            "method: POST\npath: /items\ncardinality: many\nbundle: {shape: map, format: json}\n",
        )
        .unwrap();
        let shaped = shape_request_resources(&map, &values, Some(&ids)).unwrap();
        assert_eq!(
            shaped,
            vec![serde_json::json!({"one": {"value": 1}, "two": {"value": 2}})]
        );
        assert!(
            shape_request_resources(&map, &values, Some(&["same".into(), "same".into()])).is_err()
        );

        let map_override: Operation = serde_yaml::from_str(
            "method: POST\npath: /items\ncardinality: many\nidentity_in_body: true\nbundle: {shape: map, format: json}\n",
        )
        .unwrap();
        let resource_type: ResourceType = serde_yaml::from_str(
            "id: {pointer: /id, scope: universal}\ndisplay_name: {strategy: id}\noperations: {}\n",
        )
        .unwrap();
        let value = serde_json::json!({"id": "one", "value": 1});
        let wire = outbound(&value, &resource_type, Some(&map_override)).unwrap();
        assert_eq!(
            shape_request_resources(&map_override, &[wire], Some(&["one".into()])).unwrap(),
            vec![serde_json::json!({"one": {"id": "one", "value": 1}})]
        );
    }

    #[test]
    fn list_bundles_support_json_ndjson_static_envelopes_and_multipart_carriage() {
        let values = vec![
            serde_json::json!({"id": "one"}),
            serde_json::json!({"id": "two"}),
        ];
        let json: Operation = serde_yaml::from_str(
            r#"
method: POST
path: /items
cardinality: many
bundle: {shape: list, format: json}
body: {items: []}
body_pointer: /items
"#,
        )
        .unwrap();
        assert_eq!(
            build_request_body(&json, None, None, None, Some(&values), None).unwrap(),
            Some(vec![serde_json::json!({"items": values})])
        );

        let ndjson: Operation = serde_yaml::from_str(
            "method: POST\npath: /items\ncardinality: many\nbundle: {shape: list, format: ndjson}\n",
        )
        .unwrap();
        let encoded =
            encode_payload(&values, ndjson.bundle.as_ref().map(|bundle| bundle.format)).unwrap();
        assert_eq!(
            std::str::from_utf8(&encoded).unwrap(),
            "{\"id\":\"one\"}\n{\"id\":\"two\"}\n"
        );

        let multipart: Operation = serde_yaml::from_str(
            r#"
method: POST
path: /items
cardinality: many
bundle:
  shape: list
  format: ndjson
  multipart: {name: file, filename: items.ndjson, content_type: application/x-ndjson}
"#,
        )
        .unwrap();
        assert_eq!(
            build_request_body(&multipart, None, None, None, Some(&values), None).unwrap(),
            Some(values)
        );
    }
}
