use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RepositoryLayout {
    Single,
    Multi,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PushPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncommitted: Option<GitPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untracked: Option<GitPolicy>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GitPolicy {
    Block,
    Confirm,
    Allow,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dotenv: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentConfig {
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub application: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<ProviderConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Resource Type keyed JSON pointers that tighten, but never replace, the
    /// Application's persistence redaction rules for this Target.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sensitive_fields: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSourceConfig {
    pub location: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub schema_version: u32,
    pub layout: RepositoryLayout,
    pub environments: BTreeMap<String, EnvironmentConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_source: Option<ApplicationSourceConfig>,
    #[serde(default, skip_serializing_if = "is_default_push")]
    pub push: PushPolicy,
    #[serde(default = "default_max_requests")]
    pub max_requests: usize,
}

fn is_default_push(value: &PushPolicy) -> bool {
    value == &PushPolicy::default()
}
fn default_max_requests() -> usize {
    4
}

#[derive(Clone, Debug, Serialize)]
pub struct InitResult {
    pub project: String,
    pub layout: RepositoryLayout,
    pub environments: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationDefinition {
    pub schema_version: u32,
    pub version: String,
    pub application: ApplicationIdentity,
    pub target_profile: TargetProfile,
    pub version_endpoints: Vec<VersionEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation: Option<Installation>,
    #[serde(skip)]
    pub catalogs: BTreeMap<u64, ResourceTypeCatalog>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationIdentity {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogApplicationIdentity {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceTypeCatalog {
    pub schema_version: u32,
    pub version: String,
    pub application: CatalogApplicationIdentity,
    pub resource_types: BTreeMap<String, Vec<ResourceType>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub source: String,
    pub checksum: String,
    pub taku_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetProfile {
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VersionEndpoint {
    #[serde(default = "default_get_method")]
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub pointer: String,
}

fn default_get_method() -> String {
    "GET".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub stability: ApiStability,
    pub id: Identity,
    pub display_name: DisplayName,
    #[serde(default)]
    pub namespaced: bool,
    #[serde(default = "default_write_intent")]
    pub write_intent: WriteIntent,
    #[serde(default = "default_mutation_mode")]
    pub mutation_mode: MutationMode,
    #[serde(default)]
    pub concurrency_mode: ConcurrencyMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard_pointer: Option<String>,
    #[serde(default)]
    pub missing: MissingDefaults,
    #[serde(default)]
    pub sensitive_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ResourceTypeMetadata>,
    #[serde(default)]
    pub transformations: Vec<Transformation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemProjection>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub operations: Operations,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceTypeMetadata {
    pub fields: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApiStability {
    Experimental,
    Preview,
    #[default]
    Stable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemProjection {
    pub split: FilesystemFormat,
    pub merge: FilesystemFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontmatter_markdown: Option<FrontmatterMarkdown>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemFormat {
    FrontmatterMarkdown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontmatterMarkdown {
    pub document: String,
    pub body_pointer: String,
    pub referenced_files: ReferencedFiles,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReferencedFiles {
    pub pointer: String,
    pub path_pointer: String,
    pub name_pointer: String,
    pub content_pointer: String,
    pub extension: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub pointer: String,
    pub scope: IdScope,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IdScope {
    Universal,
    Target,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pointers: Vec<String>,
    pub strategy: DisplayNameStrategy,
    #[serde(default)]
    pub unique: bool,
}

impl DisplayName {
    pub fn pointers(&self) -> impl Iterator<Item = &str> {
        self.pointers.iter().map(String::as_str).chain(
            self.pointers
                .is_empty()
                .then(|| self.pointer.as_deref().unwrap_or("/name")),
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayNameStrategy {
    Id,
    Name,
    NameId,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WriteIntent {
    Create,
    Update,
    Upsert,
}
fn default_write_intent() -> WriteIntent {
    WriteIntent::Upsert
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MutationMode {
    Replace,
    Patch,
}
fn default_mutation_mode() -> MutationMode {
    MutationMode::Replace
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MissingDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull: Option<MissingPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push: Option<MissingPolicy>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MissingPolicy {
    Conflict,
    Restore,
    Delete,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Operations {
    pub read: Option<Operation>,
    pub list: Option<Operation>,
    pub create: Option<Operation>,
    pub update: Option<Operation>,
    pub upsert: Option<Operation>,
    pub delete: Option<Operation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub method: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<NamespacePath>,
    #[serde(default, skip_serializing_if = "Cardinality::is_one")]
    pub cardinality: Cardinality,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub transformations: Vec<Transformation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_missing: Option<Outcome>,
    #[serde(default, skip_serializing_if = "is_default_response")]
    pub response: Option<ResponseDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<Bundle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unbundle: Option<PayloadFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<OperationBody>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_in_body: Option<bool>,
    #[serde(default)]
    pub skip_unidentified: bool,
    #[serde(default)]
    pub outcomes: BTreeMap<u16, Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub retry_safe: bool,
    #[serde(default, skip_serializing_if = "ConcurrencyClass::is_parallel")]
    pub concurrency: ConcurrencyClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard_header: Option<String>,
}

impl Operation {
    pub fn includes_identity_in_body(&self) -> bool {
        self.identity_in_body.unwrap_or_else(|| {
            !self.path.contains("{id}")
                && !self
                    .bundle
                    .as_ref()
                    .is_some_and(|bundle| bundle.shape == CollectionShape::Map)
        })
    }

    pub fn consumes_response_body(&self) -> bool {
        self.response
            .as_ref()
            .is_some_and(ResponseDefinition::consumes_body)
    }

    pub fn response_mapping(&self) -> Option<&ResponseMapping> {
        self.response.as_ref().and_then(ResponseDefinition::mapping)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponseDefinition {
    Kind(ResponseKind),
    Mapping(ResponseMapping),
}

impl ResponseDefinition {
    pub fn consumes_body(&self) -> bool {
        !matches!(self, Self::Kind(ResponseKind::Status))
    }

    pub fn mapping(&self) -> Option<&ResponseMapping> {
        match self {
            Self::Mapping(mapping) => Some(mapping),
            Self::Kind(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResponseKind {
    Status,
    Resource,
}

fn is_default_response(response: &Option<ResponseDefinition>) -> bool {
    matches!(
        response,
        None | Some(ResponseDefinition::Kind(ResponseKind::Status))
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseMapping {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionShape>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_pointer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_pointer: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OperationBody {
    Pointer(String),
    Template(serde_json::Value),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamespacePath {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Cardinality {
    #[default]
    One,
    Many,
}

impl Cardinality {
    fn is_one(&self) -> bool {
        *self == Self::One
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PayloadFormat {
    Json,
    Ndjson,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CollectionShape {
    List,
    Map,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub shape: CollectionShape,
    pub format: PayloadFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multipart: Option<Multipart>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Multipart {
    pub name: String,
    pub filename: String,
    pub content_type: String,
}

#[cfg(test)]
mod configuration_tests {
    use super::{
        ApplicationDefinition, Cardinality, CollectionShape, ConcurrencyClass, Operation,
        OperationBody, PayloadFormat, ResourceType, ResourceTypeMetadata, ResponseDefinition,
        ResponseKind,
    };

    #[test]
    fn parses_explicit_multipart_bundle_and_query() {
        let operation: Operation = serde_yaml::from_str(
            r#"
method: POST
path: /import
cardinality: many
query: { overwrite: true }
bundle:
  shape: list
  format: ndjson
  multipart:
    name: file
    filename: saved_objects.ndjson
    content_type: application/x-ndjson
"#,
        )
        .unwrap();

        let bundle = operation.bundle.unwrap();
        assert_eq!(bundle.shape, CollectionShape::List);
        assert_eq!(bundle.format, PayloadFormat::Ndjson);
        let multipart = bundle.multipart.unwrap();
        assert_eq!(multipart.name, "file");
        assert_eq!(multipart.filename, "saved_objects.ndjson");
        assert_eq!(multipart.content_type, "application/x-ndjson");
        assert_eq!(operation.query["overwrite"], true);
    }

    #[test]
    fn parses_static_and_pointer_operation_bodies() {
        let selector: Operation =
            serde_yaml::from_str("method: PUT\npath: /items/{id}\nbody: /policy\n").unwrap();
        assert!(matches!(
            selector.body,
            Some(OperationBody::Pointer(ref pointer)) if pointer == "/policy"
        ));

        let template: Operation =
            serde_yaml::from_str("method: POST\npath: /search\nbody: {query: '{id}'}\n").unwrap();
        assert!(matches!(template.body, Some(OperationBody::Template(_))));
    }

    #[test]
    fn identity_body_default_follows_path_and_map_binding() {
        let path_bound: Operation =
            serde_yaml::from_str("method: PUT\npath: /items/{id}\n").unwrap();
        assert!(!path_bound.includes_identity_in_body());

        let body_bound: Operation = serde_yaml::from_str("method: POST\npath: /items\n").unwrap();
        assert!(body_bound.includes_identity_in_body());

        let map: Operation = serde_yaml::from_str(
            "method: POST\npath: /items\ncardinality: many\nbundle: {shape: map, format: json}\n",
        )
        .unwrap();
        assert!(!map.includes_identity_in_body());
    }

    #[test]
    fn retry_safety_defaults_true_and_only_serializes_false() {
        let default: Operation = serde_yaml::from_str("method: GET\npath: /items\n").unwrap();
        assert!(default.retry_safe);
        assert!(
            !serde_yaml::to_string(&default)
                .unwrap()
                .contains("retry_safe")
        );

        let unsafe_operation: Operation =
            serde_yaml::from_str("method: POST\npath: /items\nretry_safe: false\n").unwrap();
        assert!(!unsafe_operation.retry_safe);
        assert!(
            serde_yaml::to_string(&unsafe_operation)
                .unwrap()
                .contains("retry_safe: false")
        );
    }

    #[test]
    fn concurrency_defaults_parallel_and_only_serializes_serial() {
        let default: Operation = serde_yaml::from_str("method: GET\npath: /items\n").unwrap();
        assert_eq!(default.concurrency, ConcurrencyClass::Parallel);
        assert!(
            !serde_yaml::to_string(&default)
                .unwrap()
                .contains("concurrency")
        );

        let serial: Operation =
            serde_yaml::from_str("method: POST\npath: /expensive\nconcurrency: serial\n").unwrap();
        assert_eq!(serial.concurrency, ConcurrencyClass::Serial);
        assert!(
            serde_yaml::to_string(&serial)
                .unwrap()
                .contains("concurrency: serial")
        );
    }

    #[test]
    fn cardinality_defaults_one_and_only_serializes_many() {
        let default: Operation = serde_yaml::from_str("method: GET\npath: /items\n").unwrap();
        assert_eq!(default.cardinality, Cardinality::One);
        assert!(
            !serde_yaml::to_string(&default)
                .unwrap()
                .contains("cardinality")
        );

        let many: Operation =
            serde_yaml::from_str("method: GET\npath: /items\ncardinality: many\n").unwrap();
        assert_eq!(many.cardinality, Cardinality::Many);
        assert!(
            serde_yaml::to_string(&many)
                .unwrap()
                .contains("cardinality: many")
        );
    }

    #[test]
    fn parses_closed_response_semantics_and_rejects_trustworthiness() {
        let omitted: Operation = serde_yaml::from_str("method: POST\npath: /items\n").unwrap();
        assert!(omitted.response.is_none());

        let status: Operation =
            serde_yaml::from_str("method: POST\npath: /items\nresponse: status\n").unwrap();
        assert!(matches!(
            status.response.as_ref(),
            Some(ResponseDefinition::Kind(ResponseKind::Status))
        ));
        assert!(!status.consumes_response_body());
        assert!(!serde_yaml::to_string(&status).unwrap().contains("response"));

        let resource: Operation =
            serde_yaml::from_str("method: POST\npath: /items\nresponse: resource\n").unwrap();
        assert!(matches!(
            resource.response.as_ref(),
            Some(ResponseDefinition::Kind(ResponseKind::Resource))
        ));
        assert!(resource.consumes_response_body());

        let mapped: Operation = serde_yaml::from_str(
            "method: GET\npath: /items\ncardinality: many\nresponse: {collection: list}\n",
        )
        .unwrap();
        assert_eq!(
            mapped
                .response_mapping()
                .and_then(|mapping| mapping.collection),
            Some(CollectionShape::List)
        );
        assert!(mapped.consumes_response_body());

        assert!(
            serde_yaml::from_str::<Operation>(
                "method: POST\npath: /items\ntrustworthy_response: true\n"
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_removed_operation_shapes() {
        for removed in [
            "bundle: multipart_ndjson",
            "default_namespace_path: /default",
            "framing: ndjson",
        ] {
            let yaml = format!("method: POST\npath: /import\ncardinality: many\n{removed}\n");
            assert!(serde_yaml::from_str::<Operation>(&yaml).is_err());
        }
    }

    #[test]
    fn requires_an_application_definition_version() {
        let yaml = r#"
schema_version: 1
application: { name: example }
target_profile: {}
version_endpoints: []
"#;
        assert!(serde_yaml::from_str::<ApplicationDefinition>(yaml).is_err());
    }

    #[test]
    fn resource_type_metadata_round_trips_without_repository_policy() {
        let yaml = r#"
id: { pointer: /id, scope: universal }
display_name: { strategy: id }
metadata:
  fields: [/created_by, /updated_at]
operations: {}
"#;

        let resource_type: ResourceType = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            resource_type.metadata,
            Some(ResourceTypeMetadata {
                fields: vec!["/created_by".into(), "/updated_at".into()],
            })
        );
        let serialized = serde_yaml::to_string(&resource_type).unwrap();
        assert!(serialized.contains("metadata:"));
        assert!(!serialized.contains("track:"));
        assert_eq!(
            serde_yaml::from_str::<ResourceType>(&serialized)
                .unwrap()
                .metadata,
            resource_type.metadata
        );
    }

    #[test]
    fn application_definition_rejects_repository_metadata_policy() {
        let yaml = r#"
schema_version: 1
version: 1.0.0
application: { name: example }
target_profile: {}
version_endpoints: []
metadata: { track: true }
"#;

        assert!(serde_yaml::from_str::<ApplicationDefinition>(yaml).is_err());
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    NotFound,
    Conflict,
    Retryable,
    Failure,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pagination {
    PageSize {
        page_parameter: String,
        size_parameter: String,
        size: usize,
        max_pages: usize,
    },
    Cursor {
        cursor_parameter: String,
        next_pointer: String,
        max_pages: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConcurrencyClass {
    Serial,
    #[default]
    Parallel,
}

impl ConcurrencyClass {
    fn is_parallel(&self) -> bool {
        *self == Self::Parallel
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConcurrencyMode {
    #[default]
    Unguarded,
    Guarded,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transformation {
    Extract {
        pointer: String,
    },
    Remove {
        pointer: String,
    },
    Omit {
        pointer: String,
    },
    Insert {
        pointer: String,
        value: serde_json::Value,
    },
    EmbeddedJson {
        pointer: String,
    },
    /// Preserve an embedded YAML document as a string on the wire while
    /// rendering it as a readable triple-quoted JSON5 value in the repository.
    /// This deliberately does not parse and re-serialize YAML: doing so would
    /// discard comments from user-authored workflow definitions.
    EmbeddedYaml {
        pointer: String,
    },
    SingletonMap {
        pointer: String,
        key_pointer: String,
        value_pointer: String,
    },
}

pub(crate) fn sensitive_field_conflicts(resource_type: &ResourceType, pointer: &str) -> bool {
    let overlaps = |required: &str| json_pointers_overlap(pointer, required);
    overlaps(&resource_type.id.pointer)
        || resource_type.display_name.pointers().any(&overlaps)
        || resource_type.transformations.iter().any(|transformation| {
            matches!(
                transformation,
                Transformation::Extract { pointer: required }
                    | Transformation::EmbeddedJson { pointer: required }
                    | Transformation::EmbeddedYaml { pointer: required }
                    if pointer == required
                        || required.starts_with(&format!("{pointer}/"))
            ) || matches!(
                transformation,
                Transformation::SingletonMap {
                    key_pointer,
                    value_pointer,
                    ..
                } if overlaps(key_pointer) || overlaps(value_pointer)
            )
        })
}

pub(crate) fn json_pointers_overlap(left: &str, right: &str) -> bool {
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}
