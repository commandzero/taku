mod metadata;
mod operation;

use self::metadata::{validate_filesystem, validate_metadata};
use self::operation::{validate_operations, validate_transformations};
use crate::canonical::TAKU_NAMESPACE_POINTER;
use crate::{ResourceType, ResourceTypeCatalog, SCHEMA_VERSION};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

pub(super) fn validate_catalog(
    expected_name: &str,
    major: u64,
    catalog: &ResourceTypeCatalog,
) -> Result<()> {
    if catalog.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Major Version Catalog schema version {}",
            catalog.schema_version
        );
    }
    if catalog.version.is_empty() {
        bail!("Major Version Catalog {major} has an empty definition version");
    }
    if catalog.application.name != expected_name {
        bail!(
            "Major Version Catalog identity {} does not match {expected_name}",
            catalog.application.name
        );
    }
    semver::VersionReq::parse(&catalog.application.version).with_context(|| {
        format!(
            "Major Version Catalog {major} has invalid Application Version constraint {}",
            catalog.application.version
        )
    })?;
    if catalog.resource_types.is_empty() {
        bail!("Major Version Catalog {major} has no Resource Types");
    }
    for (name, definitions) in &catalog.resource_types {
        validate_definitions(name, definitions, &catalog.resource_types)?;
    }
    Ok(())
}

fn validate_definitions(
    name: &str,
    definitions: &[ResourceType],
    all_types: &BTreeMap<String, Vec<ResourceType>>,
) -> Result<()> {
    if definitions.is_empty() {
        bail!("Resource Type {name} has no definitions");
    }
    for resource_type in definitions {
        validate_definition(name, resource_type, all_types)?;
    }
    Ok(())
}

fn validate_definition(
    name: &str,
    resource_type: &ResourceType,
    all_types: &BTreeMap<String, Vec<ResourceType>>,
) -> Result<()> {
    if let Some(version) = &resource_type.version {
        semver::VersionReq::parse(version).with_context(|| {
            format!("Resource Type {name} has invalid version constraint {version}")
        })?;
    }
    ResourceValidation {
        name,
        resource_type,
    }
    .validate()?;
    if let Some(dependency) = resource_type
        .dependencies
        .iter()
        .find(|dependency| !all_types.contains_key(*dependency))
    {
        bail!("Resource Type {name} depends on unknown Resource Type {dependency}");
    }
    Ok(())
}

struct ResourceValidation<'a> {
    name: &'a str,
    resource_type: &'a ResourceType,
}

impl ResourceValidation<'_> {
    fn validate(&self) -> Result<()> {
        let Self {
            name,
            resource_type,
        } = *self;
        self.validate_pointers()?;
        if resource_type.directory_pointer.is_some() && resource_type.filesystem.is_some() {
            bail!(
                "Resource Type {name} cannot combine a directory pointer with a filesystem projection"
            );
        }
        if resource_type.operations.read.is_none() && resource_type.operations.list.is_none() {
            bail!("Resource Type {name} has no observation Operation");
        }
        validate_operations(&resource_type.operations, name)?;
        validate_transformations(&resource_type.transformations, name)?;
        validate_filesystem(resource_type, name)?;
        validate_metadata(resource_type, name)?;
        if let Some(pointer) = resource_type
            .sensitive_fields
            .iter()
            .find(|pointer| crate::model::sensitive_field_conflicts(resource_type, pointer))
        {
            bail!("Sensitive Field {pointer} overlaps required canonical state for {name}");
        }
        self.validate_write_intent()
    }

    fn validate_pointers(&self) -> Result<()> {
        let Self {
            name,
            resource_type,
        } = *self;
        if !resource_type.id.pointer.starts_with('/') {
            bail!("Resource Type {name} has an invalid identity pointer");
        }
        if reserved_canonical_pointer(&resource_type.id.pointer) {
            bail!("Resource Type {name} identity pointer is reserved for Taku state");
        }
        self.validate_display_name()?;
        let reserved = resource_type
            .display_name
            .pointers()
            .chain(resource_type.guard_pointer.as_deref())
            .chain(resource_type.sensitive_fields.iter().map(String::as_str))
            .chain(
                resource_type
                    .transformations
                    .iter()
                    .flat_map(transformation_pointers),
            );
        if reserved.into_iter().any(reserved_canonical_pointer) {
            bail!("Resource Type {name} uses a pointer in Taku's reserved namespace");
        }
        if let Some(pointer) = &resource_type.directory_pointer {
            self.validate_directory_pointer(pointer)?;
        }
        Ok(())
    }

    fn validate_display_name(&self) -> Result<()> {
        let name = self.name;
        let display_name = &self.resource_type.display_name;
        if display_name.pointer.is_some() && !display_name.pointers.is_empty() {
            bail!("Resource Type {name} Display Name cannot define both pointer and pointers");
        }
        if display_name
            .pointers()
            .any(|pointer| !pointer.starts_with('/'))
        {
            bail!("Resource Type {name} has an invalid Display Name pointer");
        }
        Ok(())
    }

    fn validate_directory_pointer(&self, pointer: &str) -> Result<()> {
        let name = self.name;
        if !valid_json_pointer(pointer) {
            bail!("Resource Type {name} has an invalid directory pointer");
        }
        if reserved_canonical_pointer(pointer) {
            bail!("Resource Type {name} directory pointer is reserved for Taku state");
        }
        Ok(())
    }

    fn validate_write_intent(&self) -> Result<()> {
        let Self {
            name,
            resource_type,
        } = *self;
        if match resource_type.write_intent {
            crate::WriteIntent::Create => resource_type.operations.create.is_none(),
            crate::WriteIntent::Update => resource_type.operations.update.is_none(),
            crate::WriteIntent::Upsert => {
                resource_type.operations.upsert.is_none()
                    && (resource_type.operations.create.is_none()
                        || resource_type.operations.update.is_none())
            }
        } {
            bail!("Resource Type {name} cannot enforce configured Write Intent");
        }
        Ok(())
    }
}

pub(super) fn valid_json_pointer(pointer: &str) -> bool {
    if !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(character) = chars.next() {
        if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

pub(super) fn reserved_canonical_pointer(pointer: &str) -> bool {
    pointer == TAKU_NAMESPACE_POINTER || pointer.starts_with(&format!("{TAKU_NAMESPACE_POINTER}/"))
}

fn transformation_pointers(transformation: &crate::Transformation) -> Vec<&str> {
    match transformation {
        crate::Transformation::Extract { pointer }
        | crate::Transformation::Remove { pointer }
        | crate::Transformation::Omit { pointer }
        | crate::Transformation::Insert { pointer, .. }
        | crate::Transformation::EmbeddedJson { pointer }
        | crate::Transformation::EmbeddedYaml { pointer } => vec![pointer],
        crate::Transformation::SingletonMap {
            pointer,
            key_pointer,
            value_pointer,
        } => vec![pointer, key_pointer, value_pointer],
    }
}

#[cfg(test)]
mod metadata_validation_tests {
    use super::ResourceValidation;

    fn validate_resource_type(
        resource_type: &crate::ResourceType,
        name: &str,
    ) -> anyhow::Result<()> {
        ResourceValidation {
            name,
            resource_type,
        }
        .validate()
    }

    fn resource_type(extra: &str) -> crate::ResourceType {
        serde_yaml::from_str(&format!(
            r#"
id: {{ pointer: /id, scope: universal }}
display_name: {{ strategy: id }}
{extra}
operations:
  read: {{ method: GET, path: "/items/{{id}}" }}
  upsert: {{ method: PUT, path: "/items/{{id}}" }}
"#
        ))
        .unwrap()
    }

    #[test]
    fn accepts_distinct_canonical_metadata_pointers() {
        validate_resource_type(
            &resource_type("metadata: { fields: [/created_by, /updated_at] }"),
            "item",
        )
        .unwrap();
    }

    #[test]
    fn rejects_invalid_or_ambiguous_metadata_declarations() {
        for extra in [
            "metadata: { fields: [] }",
            "metadata: { fields: [''] }",
            "metadata: { fields: [/bad~2escape] }",
            "metadata: { fields: [/audit, /audit] }",
            "metadata: { fields: [/audit, /audit/user] }",
            "metadata: { fields: [/id] }",
            "metadata: { fields: [/name] }",
            "sensitive_fields: [/secret]\nmetadata: { fields: [/secret/owner] }",
            "transformations: [{ kind: remove, pointer: /server }]\nmetadata: { fields: [/server/time] }",
            "metadata: { fields: [/_taku/provenance] }",
            "transformations: [{ kind: remove, pointer: /_taku/provenance }]",
        ] {
            assert!(
                validate_resource_type(&resource_type(extra), "item").is_err(),
                "{extra}"
            );
        }
        let mut reserved_id = resource_type("");
        reserved_id.id.pointer = "/_taku/id".into();
        assert!(validate_resource_type(&reserved_id, "item").is_err());
    }
}
