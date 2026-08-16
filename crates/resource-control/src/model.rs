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
    pub application: ApplicationIdentity,
    pub target_profile: TargetProfile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation: Option<Installation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationIdentity {
    pub name: String,
    pub version: String,
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
    #[serde(default)]
    pub fact_probes: Vec<FactProbe>,
    pub resource_types: BTreeMap<String, ResourceType>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactProbe {
    pub name: String,
    pub operation: Operation,
    pub pointer: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceType {
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
    #[serde(default)]
    pub transformations: Vec<Transformation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemProjection>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub variants: Vec<ResourceVariant>,
    pub operations: Operations,
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
    pub default_namespace_path: Option<String>,
    pub cardinality: Cardinality,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub transformations: Vec<Transformation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_missing: Option<Outcome>,
    #[serde(alias = "request_framing", skip_serializing_if = "Option::is_none")]
    pub bundle: Option<PayloadFormat>,
    #[serde(alias = "response_framing", skip_serializing_if = "Option::is_none")]
    pub unbundle: Option<PayloadFormat>,
    /// Backward-compatible shorthand used by schema version 1 definitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framing: Option<PayloadFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_pointer: Option<String>,
    #[serde(default)]
    pub skip_unidentified: bool,
    #[serde(default)]
    pub outcomes: BTreeMap<u16, Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    #[serde(default)]
    pub retry_safe: bool,
    #[serde(default)]
    pub trustworthy_response: bool,
    #[serde(default)]
    pub concurrency: ConcurrencyClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard_header: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Cardinality {
    One,
    Many,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PayloadFormat {
    Json,
    Ndjson,
    MultipartNdjson,
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
    #[default]
    Serial,
    Parallel,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConcurrencyMode {
    #[default]
    Unguarded,
    Guarded,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceVariant {
    pub name: String,
    #[serde(default)]
    pub facts: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operations: Option<Operations>,
    #[serde(default)]
    pub transformations: Vec<Transformation>,
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
    Frame {
        pointer: String,
    },
    SingletonMap {
        pointer: String,
        key_pointer: String,
        value_pointer: String,
    },
}

pub(crate) fn sensitive_field_conflicts(resource_type: &ResourceType, pointer: &str) -> bool {
    let overlaps = |required: &str| {
        pointer == required
            || required.starts_with(&format!("{pointer}/"))
            || pointer.starts_with(&format!("{required}/"))
    };
    overlaps(&resource_type.id.pointer)
        || resource_type.display_name.pointers().any(&overlaps)
        || resource_type.transformations.iter().any(|transformation| {
            matches!(
                transformation,
                Transformation::Extract { pointer: required }
                    | Transformation::EmbeddedJson { pointer: required }
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
