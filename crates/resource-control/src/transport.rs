mod pointers;
mod request;
mod response;
mod transform;

#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod namespace_path_tests;

pub use pointers::{insert_pointer, remove_pointer};
pub use transform::outbound;
pub(crate) use transform::{inbound, without_metadata};

use request::{attach_body, build_request_body, operation_path, render_path};
use response::map_response;

use crate::{ApplicationDefinition, Operation, ResourceType, TargetConfig, VersionEndpoint};
use anyhow::{Context, Result, bail};
use redact::Secret;
use reqwest::Method;
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::Value;
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
impl<'a> OperationInput<'a> {
    fn template_context(&self) -> Option<&'a Value> {
        self.context
            .or_else(|| self.body.and_then(|values| values.first()))
    }
}

pub const INTERNAL_GUARD_POINTER: &str = "/_taku/guard";
pub const INTERNAL_CURSOR_POINTER: &str = "/_taku/cursor";

pub fn execute_version_endpoint(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    endpoint: &VersionEndpoint,
    auth: &BTreeMap<String, Secret<String>>,
) -> Result<Value> {
    let path = render_path(&endpoint.path, None, None, None);
    let request = http_request(target, app, &endpoint.method, &path, auth)?;
    let request = apply_headers(request, &endpoint.headers, auth);
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
        input.template_context(),
    );
    let body = build_request_body(operation, input)?;
    let mut request = http_request(target, app, &operation.method, &path, auth)?;
    if !operation.query.is_empty() {
        request = request.query(&operation.query);
    }
    let request = apply_headers(request, &operation.headers, auth);
    let request = attach_body(request, operation, body.as_deref())?;
    let response = match request.send() {
        Ok(response) => response,
        Err(error) if error.is_timeout() || error.is_connect() => {
            return Ok(RemoteResult::Uncertain);
        }
        Err(_) => return Ok(RemoteResult::Failure("remote request failed".into())),
    };
    map_response(response, operation, resource_type, input)
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
    let attempts = if operation.retry_safe { 3 } else { 1 };
    let mut result = execute(target, app, resource_type, operation, input, auth)?;
    for _ in 1..attempts {
        if !matches!(result, RemoteResult::Retryable | RemoteResult::Uncertain) {
            break;
        }
        result = execute(target, app, resource_type, operation, input, auth)?;
    }
    Ok(result)
}

fn http_request(
    target: &TargetConfig,
    app: &ApplicationDefinition,
    method: &str,
    path: &str,
    auth: &BTreeMap<String, Secret<String>>,
) -> Result<RequestBuilder> {
    let base_url = auth
        .get("url")
        .map(|value| value.expose_secret().as_str())
        .unwrap_or(&target.url);
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let method = Method::from_bytes(method.as_bytes()).context("invalid configured HTTP method")?;
    let mut request = client.request(method, &url);
    for (name, value) in &app.target_profile.headers {
        request = request.header(name, value);
    }
    for (name, value) in &target.headers {
        request = request.header(name, value);
    }
    Ok(request)
}

fn apply_headers(
    mut request: RequestBuilder,
    configured: &BTreeMap<String, String>,
    auth: &BTreeMap<String, Secret<String>>,
) -> RequestBuilder {
    for (name, value) in configured {
        request = request.header(name, value);
    }
    for (name, value) in auth.iter().filter(|(name, _)| name.as_str() != "url") {
        request = request.header(name, value.expose_secret());
    }
    request
}
