use super::{OperationInput, insert_pointer};
use crate::{Operation, PayloadFormat};
use anyhow::{Context, Result, bail};
use reqwest::blocking::RequestBuilder;
use serde_json::{Map, Value};
use std::borrow::Cow;

pub(super) fn encode_payload(values: &[Value], format: Option<PayloadFormat>) -> Result<Vec<u8>> {
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

pub(super) fn build_request_body(
    operation: &Operation,
    input: OperationInput<'_>,
) -> Result<Option<Vec<Value>>> {
    let shaped_resources = input
        .body
        .map(|resources| shape_request_resources(operation, resources, input.resource_ids))
        .transpose()?;
    let Some(crate::OperationBody::Template(template)) = operation.body.as_ref() else {
        return Ok(shaped_resources);
    };
    let mut body = render_body_template(
        template,
        input.namespace,
        input.id,
        input.template_context(),
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

pub(super) fn shape_request_resources(
    operation: &Operation,
    resources: &[Value],
    resource_ids: Option<&[String]>,
) -> Result<Vec<Value>> {
    match operation.bundle.as_ref() {
        Some(bundle) if bundle.shape == crate::CollectionShape::Map => {
            map_bundle(resources, resource_ids)
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

fn map_bundle(resources: &[Value], resource_ids: Option<&[String]>) -> Result<Vec<Value>> {
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

fn render_body_template(
    template: &Value,
    namespace: Option<&str>,
    id: Option<&str>,
    resource: Option<&Value>,
) -> Value {
    match template {
        Value::String(value) => Value::String(render_tokens(
            value,
            TemplateValues {
                namespace,
                id,
                resource,
            },
            str::to_owned,
        )),
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

pub(super) fn operation_path<'a>(
    operation: &'a Operation,
    namespace: Option<&str>,
) -> Cow<'a, str> {
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

pub(super) fn render_path(
    template: &str,
    namespace: Option<&str>,
    id: Option<&str>,
    value: Option<&Value>,
) -> String {
    render_tokens(
        template,
        TemplateValues {
            namespace,
            id,
            resource: value,
        },
        |token| urlencoding::encode(token).into_owned(),
    )
}

struct TemplateValues<'a> {
    namespace: Option<&'a str>,
    id: Option<&'a str>,
    resource: Option<&'a Value>,
}

fn render_tokens(
    template: &str,
    values: TemplateValues<'_>,
    encode: impl Fn(&str) -> String,
) -> String {
    let mut rendered = template.replace("{id}", &encode(values.id.unwrap_or("")));
    rendered = rendered.replace("{namespace}", &encode(values.namespace.unwrap_or("")));
    let fields = values
        .resource
        .and_then(Value::as_object)
        .into_iter()
        .flatten();
    for (name, value) in fields.filter_map(|(name, value)| Some((name, value.as_str()?))) {
        rendered = rendered.replace(&format!("{{{name}}}"), &encode(value));
    }
    rendered
}

pub(super) fn attach_body(
    request: RequestBuilder,
    operation: &Operation,
    values: Option<&[Value]>,
) -> Result<RequestBuilder> {
    let Some(values) = values else {
        return Ok(request);
    };
    let format = operation.bundle.as_ref().map(|bundle| bundle.format);
    let body = encode_payload(values, format)?;
    let request = if let Some(multipart) = operation
        .bundle
        .as_ref()
        .and_then(|bundle| bundle.multipart.as_ref())
    {
        let part = reqwest::blocking::multipart::Part::bytes(body)
            .file_name(multipart.filename.clone())
            .mime_str(&multipart.content_type)?;
        request
            .multipart(reqwest::blocking::multipart::Form::new().part(multipart.name.clone(), part))
    } else if format == Some(PayloadFormat::Ndjson) {
        request
            .body(body)
            .header("content-type", "application/x-ndjson")
    } else if values.len() == 1 {
        request.json(&values[0])
    } else {
        request.json(values)
    };
    Ok(request)
}
