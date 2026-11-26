use super::{reserved_canonical_pointer, transformation_pointers, valid_json_pointer};
use crate::{
    Bundle, Cardinality, CollectionShape, NamespacePath, Operation, OperationBody, PayloadFormat,
    ResponseMapping,
};
use anyhow::{Context, Result, bail};

pub(super) fn validate_operations(operations: &crate::Operations, owner: &str) -> Result<()> {
    for (name, operation, observes_response, requires_bundle) in [
        ("read", operations.read.as_ref(), true, false),
        ("list", operations.list.as_ref(), true, false),
        ("create", operations.create.as_ref(), false, true),
        ("update", operations.update.as_ref(), false, true),
        ("upsert", operations.upsert.as_ref(), false, true),
        ("delete", operations.delete.as_ref(), false, false),
    ] {
        let Some(operation) = operation else { continue };
        validate_response_use(operation, owner, name, observes_response)?;
        let label = format!("Resource Type {owner} {name}");
        OperationValidation {
            operation,
            owner: &label,
        }
        .validate(observes_response, requires_bundle)?;
    }
    Ok(())
}

fn validate_response_use(
    operation: &Operation,
    owner: &str,
    name: &str,
    observes_response: bool,
) -> Result<()> {
    if !operation.consumes_response_body() {
        return Ok(());
    }
    match (name, observes_response, operation.cardinality) {
        ("delete", _, _) => {
            bail!("Resource Type {owner} delete cannot consume a Resource response")
        }
        (_, false, Cardinality::Many) => {
            bail!(
                "Resource Type {owner} {name} many-Resource mutation cannot consume a Resource response"
            );
        }
        _ => Ok(()),
    }
}

struct OperationValidation<'a> {
    operation: &'a Operation,
    owner: &'a str,
}

impl OperationValidation<'_> {
    fn validate(&self, resource_response_by_definition: bool, requires_bundle: bool) -> Result<()> {
        let Self { operation, owner } = *self;
        let observes_response =
            resource_response_by_definition || operation.consumes_response_body();
        reqwest::Method::from_bytes(operation.method.as_bytes())
            .with_context(|| format!("{owner} has an invalid HTTP method"))?;
        if !operation.path.starts_with('/') {
            bail!("{owner} Operation path must begin with /");
        }
        if let Some(namespace) = &operation.namespace {
            self.validate_namespace_path(namespace)?;
        }
        self.validate_body()?;
        self.validate_response(observes_response)?;
        self.validate_bundle(requires_bundle)?;
        validate_transformations(&operation.transformations, owner)
    }

    fn validate_namespace_path(&self, namespace: &NamespacePath) -> Result<()> {
        let owner = self.owner;
        let parts = [
            ("prefix", namespace.prefix.as_deref()),
            ("suffix", namespace.suffix.as_deref()),
        ];
        if parts.iter().all(|(_, part)| part.is_none()) {
            bail!("{owner} Operation namespace must define a prefix or suffix");
        }
        let invalid = parts
            .into_iter()
            .find_map(|(label, part)| part.filter(|value| !value.starts_with('/')).map(|_| label));
        if let Some(label) = invalid {
            bail!("{owner} Operation namespace {label} must begin with /");
        }
        if !parts
            .into_iter()
            .filter_map(|(_, part)| part)
            .any(|part| part.contains("{namespace}"))
        {
            bail!("{owner} Operation namespace prefix or suffix must contain {{namespace}}");
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        let Self { operation, owner } = *self;
        if operation
            .body_pointer
            .as_deref()
            .is_some_and(|pointer| !valid_body_pointer(operation, pointer))
        {
            bail!("{owner} Operation body_pointer requires a static body and a JSON pointer");
        }
        if let Some(OperationBody::Pointer(pointer)) = &operation.body {
            self.validate_body_selector(pointer)?;
        }
        Ok(())
    }

    fn validate_body_selector(&self, pointer: &str) -> Result<()> {
        let owner = self.owner;
        if !valid_json_pointer(pointer) {
            bail!("{owner} Operation body selector is not a JSON pointer");
        }
        if reserved_canonical_pointer(pointer) {
            bail!("{owner} Operation body selector uses Taku's reserved namespace");
        }
        Ok(())
    }

    fn validate_response(&self, observes_response: bool) -> Result<()> {
        let Self { operation, owner } = *self;
        if operation
            .extract_missing
            .is_some_and(|outcome| matches!(outcome, crate::Outcome::Success))
        {
            bail!("{owner} Operation extract_missing cannot map to success");
        }
        if observes_response
            && matches!(
                operation.response.as_ref(),
                Some(crate::ResponseDefinition::Kind(crate::ResponseKind::Status))
            )
        {
            bail!("{owner} Resource response cannot use status-only handling");
        }
        let mapping = operation.response_mapping();
        if let Some(response) = mapping {
            self.validate_mapping(response)?;
        }
        let collection = mapping.and_then(|response| response.collection);
        match (observes_response, operation.cardinality, collection) {
            (true, Cardinality::Many, None) => {
                bail!("{owner} many-Resource response requires an explicit collection shape");
            }
            _ => Ok(()),
        }
    }

    fn validate_mapping(&self, response: &ResponseMapping) -> Result<()> {
        let Self { operation, owner } = *self;
        let pointers = [
            response.identity_pointer.as_deref(),
            response.resource_pointer.as_deref(),
        ];
        if pointers
            .into_iter()
            .flatten()
            .any(|pointer| !valid_json_pointer(pointer))
        {
            bail!("{owner} Operation response has an invalid JSON pointer");
        }
        if operation.cardinality == Cardinality::One && response.collection.is_some() {
            bail!("{owner} one-Resource Operation cannot declare a response collection");
        }
        if response.collection == Some(CollectionShape::Map) {
            self.validate_map_response(response)?;
        }
        Ok(())
    }

    fn validate_map_response(&self, response: &ResponseMapping) -> Result<()> {
        let owner = self.owner;
        if response.identity_pointer.is_some() {
            bail!("{owner} map response identity comes from its keys");
        }
        if self.operation.unbundle == Some(PayloadFormat::Ndjson) {
            bail!("{owner} map response does not support NDJSON");
        }
        Ok(())
    }

    fn validate_bundle(&self, requires_bundle: bool) -> Result<()> {
        let Self { operation, owner } = *self;
        match &operation.bundle {
            Some(bundle) => self.validate_bundle_shape(bundle),
            None if requires_bundle && operation.cardinality == Cardinality::Many => {
                bail!("{owner} many-Resource mutation requires an explicit bundle shape");
            }
            None => Ok(()),
        }
    }

    fn validate_bundle_shape(&self, bundle: &Bundle) -> Result<()> {
        let Self { operation, owner } = *self;
        if operation.cardinality != Cardinality::Many {
            bail!("{owner} bundle requires many cardinality");
        }
        if bundle.format == PayloadFormat::Ndjson {
            self.validate_ndjson_bundle(bundle)?;
        }
        if bundle.multipart.as_ref().is_some_and(|part| {
            [&part.name, &part.filename, &part.content_type]
                .into_iter()
                .any(String::is_empty)
        }) {
            bail!("{owner} Operation multipart bundle fields cannot be empty");
        }
        Ok(())
    }

    fn validate_ndjson_bundle(&self, bundle: &Bundle) -> Result<()> {
        let Self { operation, owner } = *self;
        if bundle.shape == CollectionShape::Map {
            bail!("{owner} map bundle does not support NDJSON");
        }
        if matches!(operation.body, Some(OperationBody::Template(_))) {
            bail!("{owner} NDJSON bundle cannot use a static body envelope");
        }
        Ok(())
    }
}

fn valid_body_pointer(operation: &Operation, pointer: &str) -> bool {
    matches!(operation.body, Some(OperationBody::Template(_))) && pointer.starts_with('/')
}

pub(super) fn validate_transformations(
    transformations: &[crate::Transformation],
    owner: &str,
) -> Result<()> {
    for transformation in transformations {
        let pointers = transformation_pointers(transformation);
        if pointers.iter().any(|pointer| !pointer.starts_with('/')) {
            bail!("{owner} Transformation has an invalid JSON pointer");
        }
        if pointers
            .iter()
            .any(|pointer| reserved_canonical_pointer(pointer))
        {
            bail!("{owner} Transformation uses Taku's reserved namespace");
        }
        if let crate::Transformation::SingletonMap {
            key_pointer,
            value_pointer,
            ..
        } = transformation
            && crate::model::json_pointers_overlap(key_pointer, value_pointer)
        {
            bail!("{owner} Singleton Map output pointers overlap");
        }
    }
    Ok(())
}

#[cfg(test)]
mod operation_shape_validation_tests {
    use super::{OperationValidation, validate_operations};

    fn validate_operation(
        operation: &crate::Operation,
        owner: &str,
        observes_response: bool,
        requires_bundle: bool,
    ) -> anyhow::Result<()> {
        OperationValidation { operation, owner }.validate(observes_response, requires_bundle)
    }

    fn operation(yaml: &str) -> crate::Operation {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn assert_valid(yaml: &str, observes_response: bool) {
        validate_operation(&operation(yaml), "test", observes_response, false).unwrap();
    }

    fn assert_invalid(yaml: &str, observes_response: bool) {
        assert!(
            validate_operation(&operation(yaml), "test", observes_response, false).is_err(),
            "expected invalid Operation:\n{yaml}"
        );
    }

    #[test]
    fn accepts_direct_list_map_and_body_selector_definitions() {
        assert_valid("method: GET\npath: /items/{id}\nresponse: resource\n", true);
        assert_valid("method: POST\npath: /items\nresponse: status\n", false);
        assert_valid("method: POST\npath: /items\nresponse: resource\n", false);
        assert_valid(
            "method: GET\npath: /items/{id}\nresponse: {resource_pointer: /item}\n",
            true,
        );
        assert_valid(
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: list, identity_pointer: /name, resource_pointer: /item}\n",
            true,
        );
        assert_valid(
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: map}\n",
            true,
        );
        assert_valid("method: PUT\npath: /items/{id}\nbody: /policy\n", false);
    }

    #[test]
    fn rejects_incompatible_response_mappings() {
        assert_invalid("method: GET\npath: /items/{id}\nresponse: status\n", true);
        for yaml in [
            "method: GET\npath: /items/{id}\nresponse: {collection: list}\n",
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: map, identity_pointer: /id}\n",
            "method: GET\npath: /items\ncardinality: many\nunbundle: ndjson\nresponse: {collection: map}\n",
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: list, resource_pointer: item}\n",
            "method: GET\npath: /items\ncardinality: many\n",
        ] {
            assert_invalid(yaml, true);
        }
    }

    #[test]
    fn rejects_incompatible_request_shapes() {
        for yaml in [
            "method: POST\npath: /items\nbundle: {shape: list, format: json}\n",
            "method: POST\npath: /items\ncardinality: many\nbundle: {shape: map, format: ndjson}\n",
            "method: POST\npath: /items\ncardinality: many\nbundle: {shape: list, format: ndjson}\nbody: {items: []}\n",
            "method: PUT\npath: /items/{id}\nbody: policy\n",
            "method: PUT\npath: /items/{id}\nbody: /policy\nbody_pointer: /item\n",
            "method: PUT\npath: /items/{id}\nbody: /_taku/id\n",
            "method: PUT\npath: /items/{id}\ntransformations: [{ kind: omit, pointer: /_taku/provenance }]\n",
        ] {
            assert_invalid(yaml, false);
        }
        assert!(
            validate_operation(
                &operation("method: POST\npath: /items\ncardinality: many\n"),
                "test",
                false,
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn restricts_resource_responses_to_observations_and_single_resource_mutations() {
        let valid: crate::Operations =
            serde_yaml::from_str("create: {method: POST, path: /items, response: resource}\n")
                .unwrap();
        validate_operations(&valid, "test").unwrap();

        for yaml in [
            "read: {method: GET, path: '/items/{id}', response: status}\n",
            "delete: {method: DELETE, path: '/items/{id}', response: resource}\n",
            "create: {method: POST, path: /items, cardinality: many, response: {collection: list}, bundle: {shape: list, format: json}}\n",
        ] {
            let operations: crate::Operations = serde_yaml::from_str(yaml).unwrap();
            assert!(validate_operations(&operations, "test").is_err());
        }
    }

    #[test]
    fn rejects_legacy_frame_transformation() {
        let yaml = r#"
method: PUT
path: /items/{id}
transformations:
  - { kind: frame, pointer: /item }
"#;
        assert!(serde_yaml::from_str::<crate::Operation>(yaml).is_err());
    }
}
