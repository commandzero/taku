use super::request::{
    build_request_body, encode_payload, operation_path, render_path, shape_request_resources,
};
use super::response::{decode_response, reconcile_identity};
use super::{OperationInput, inbound, outbound};
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
    let operation: Operation = serde_yaml::from_str("method: PUT\npath: /items/{id}\n").unwrap();
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

    let path_default: Operation = serde_yaml::from_str("method: PUT\npath: /items/{id}\n").unwrap();
    assert_eq!(
        outbound(&value, &resource_type, Some(&path_default)).unwrap(),
        serde_json::json!({"identity": {}, "value": 1})
    );

    let path_override: Operation =
        serde_yaml::from_str("method: PUT\npath: /items/{id}\nidentity_in_body: true\n").unwrap();
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
    assert!(shape_request_resources(&map, &values, Some(&["same".into(), "same".into()])).is_err());

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
        build_request_body(
            &json,
            OperationInput {
                body: Some(&values),
                ..Default::default()
            }
        )
        .unwrap(),
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
        build_request_body(
            &multipart,
            OperationInput {
                body: Some(&values),
                ..Default::default()
            }
        )
        .unwrap(),
        Some(values)
    );
}
