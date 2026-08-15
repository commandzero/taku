use crate::canonical::{pointer_string, sort_value};
use crate::{ApplicationDefinition, Operation, Outcome, PayloadFormat, ResourceType, TargetConfig};
use anyhow::{Context, Result, bail};
use redact::Secret;
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value};
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
}
pub const INTERNAL_GUARD_POINTER: &str = "/_taku_internal_guard";
pub const INTERNAL_CURSOR_POINTER: &str = "/_taku_internal_cursor";

pub fn execute_probe(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    operation: &Operation,
    auth: &BTreeMap<String, Secret<String>>,
) -> Result<Value> {
    let base_url = auth
        .get("url")
        .map(|value| value.expose_secret().as_str())
        .unwrap_or(&target.url);
    let path = render_path(operation_path(operation, None), None, None, None);
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let method = Method::from_bytes(operation.method.as_bytes())
        .context("invalid configured HTTP method")?;
    let mut attempts = 0;
    let response = loop {
        attempts += 1;
        let mut request = client.request(method.clone(), &url);
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
        match request.send() {
            Ok(response)
                if operation.retry_safe
                    && attempts < 3
                    && (response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error()) =>
            {
                continue;
            }
            Ok(response) => break response,
            Err(error)
                if operation.retry_safe
                    && attempts < 3
                    && (error.is_timeout() || error.is_connect()) =>
            {
                continue;
            }
            Err(_) => bail!("Target Fact probe failed"),
        }
    };
    if !response.status().is_success() {
        bail!(
            "Target Fact probe returned HTTP {}",
            response.status().as_u16()
        );
    }
    let bytes = response
        .bytes()
        .map_err(|_| anyhow::anyhow!("Target Fact probe response failed"))?;
    json5::from_str(
        std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("Target Fact probe response is not UTF-8"))?,
    )
    .map_err(|_| anyhow::anyhow!("Target Fact probe response is invalid"))
}

pub fn execute(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    resource_type: &ResourceType,
    operation: &Operation,
    input: OperationInput<'_>,
    auth: &BTreeMap<String, Secret<String>>,
) -> Result<RemoteResult> {
    let path = render_path(
        operation_path(operation, input.namespace),
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
        let framing = operation.bundle.or(operation.framing);
        request = if framing == Some(PayloadFormat::Ndjson) {
            let mut framed = String::new();
            for value in values {
                framed.push_str(&serde_json::to_string(value)?);
                framed.push('\n');
            }
            request
                .body(framed)
                .header("content-type", "application/x-ndjson")
        } else if framing == Some(PayloadFormat::MultipartNdjson) {
            let mut framed = String::new();
            for value in values {
                framed.push_str(&serde_json::to_string(value)?);
                framed.push('\n');
            }
            let part = reqwest::blocking::multipart::Part::bytes(framed.into_bytes())
                .file_name("export.ndjson")
                .mime_str("application/x-ndjson")?;
            request.multipart(reqwest::blocking::multipart::Form::new().part("file", part))
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
    map_response(response, operation, resource_type, input.id)
        .context("Transformation Conflict while converting successful response")
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
) -> Result<Option<Vec<Value>>> {
    let Some(template) = operation.body.as_ref() else {
        return Ok(resources.map(<[Value]>::to_vec));
    };
    let mut body = render_body_template(
        template,
        namespace,
        id,
        context.or_else(|| resources.and_then(|values| values.first())),
    );
    if let Some(resources) = resources
        && let Some(pointer) = operation.body_pointer.as_deref()
    {
        let inserted = if operation.cardinality == crate::Cardinality::Many {
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
            let bytes = response
                .bytes()
                .map_err(|_| anyhow::anyhow!("failed to read successful remote response"))?;
            if bytes.is_empty() {
                return Ok(RemoteResult::Success(Vec::new()));
            }
            let mut values = parse_values(&bytes, operation.unbundle.or(operation.framing))
                .map_err(|_| {
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
                }
                .context("configured response extraction did not match")?;
                values = vec![extracted];
            }
            values = expand_many(values, resource_type, id, operation.skip_unidentified)?;
            for value in &mut values {
                let guard = resource_type
                    .guard_pointer
                    .as_deref()
                    .and_then(|pointer| pointer_string(value, pointer));
                if let Some(id) = id
                    && pointer_string(value, &resource_type.id.pointer).is_none()
                {
                    insert_pointer(value, &resource_type.id.pointer, Value::String(id.into()))?;
                }
                for pointer in &resource_type.sensitive_fields {
                    remove_pointer(value, pointer)?;
                }
                apply_inbound(value, resource_type)?;
                if let Some(guard) = guard {
                    insert_pointer(value, INTERNAL_GUARD_POINTER, Value::String(guard))?;
                }
                *value = sort_value(value);
            }
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

fn parse_values(bytes: &[u8], framing: Option<PayloadFormat>) -> Result<Vec<Value>> {
    let text = std::str::from_utf8(bytes).context("remote response is not UTF-8")?;
    if framing == Some(PayloadFormat::Ndjson) {
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

fn expand_many(
    values: Vec<Value>,
    resource_type: &ResourceType,
    requested_id: Option<&str>,
    skip_unidentified: bool,
) -> Result<Vec<Value>> {
    if requested_id.is_some() {
        return Ok(if skip_unidentified {
            values
                .into_iter()
                .filter(|value| pointer_string(value, &resource_type.id.pointer).is_some())
                .collect()
        } else {
            values
        });
    }
    let mut out = Vec::new();
    for value in values {
        match value {
            Value::Array(items) => out.extend(items),
            Value::Object(map)
                if pointer_string(&Value::Object(map.clone()), &resource_type.id.pointer)
                    .is_none() =>
            {
                for (id, mut item) in map {
                    if !item.is_object() {
                        continue;
                    }
                    insert_pointer(&mut item, &resource_type.id.pointer, Value::String(id))?;
                    out.push(item);
                }
            }
            value => out.push(value),
        }
    }
    let mut identified = Vec::new();
    for value in out {
        let Some(_id) = pointer_string(&value, &resource_type.id.pointer) else {
            if skip_unidentified {
                continue;
            }
            bail!("remote Resource has no configured ID");
        };
        identified.push(value);
    }
    Ok(identified)
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
            crate::Transformation::Frame { pointer } => {
                let document = value.clone();
                *value = Value::Object(Map::new());
                insert_pointer(value, pointer, document)?;
            }
        }
    }
    Ok(())
}

pub fn outbound(
    value: &Value,
    resource_type: &ResourceType,
    operation: Option<&Operation>,
) -> Result<Value> {
    let mut value = value.clone();
    apply_outbound(&mut value, &resource_type.transformations)?;
    if let Some(operation) = operation {
        apply_outbound(&mut value, &operation.transformations)?;
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
            crate::Transformation::Frame { pointer } => {
                *value = value
                    .pointer(pointer)
                    .cloned()
                    .context("outbound framing pointer did not match")?;
            }
            crate::Transformation::Insert { pointer, .. } => remove_pointer(value, pointer)?,
            crate::Transformation::Omit { pointer } => remove_pointer(value, pointer)?,
            crate::Transformation::Remove { .. } => {}
        }
    }
    Ok(())
}

fn operation_path<'a>(operation: &'a Operation, namespace: Option<&str>) -> &'a str {
    if namespace == Some("default") {
        operation
            .default_namespace_path
            .as_deref()
            .unwrap_or(&operation.path)
    } else {
        &operation.path
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
        let Some(next) = current.get_mut(&token) else {
            return Ok(());
        };
        current = next;
    }
    if let Some(map) = current.as_object_mut() {
        map.remove(&last.replace("~1", "/").replace("~0", "~"));
    }
    Ok(())
}
