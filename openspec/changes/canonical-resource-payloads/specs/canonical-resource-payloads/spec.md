## Purpose

Defines how API response collections become self-contained Canonical Resources and how those Resources become single- or many-resource mutation payloads without preserving API collection envelopes in Git.

## ADDED Requirements

### Requirement: Canonical Resources use the single-resource shape
The system SHALL persist the representation of one Resource rather than an envelope introduced by a read or list response. The configured Resource ID SHALL remain present at the Resource Type identity pointer in every persisted Canonical Resource, even when the API represents that identity outside the single-resource body.

#### Scenario: Enveloped list item becomes one Canonical Resource
- **WHEN** a list response item contains identity at `/name` and its Resource body at `/component_template`
- **THEN** the system persists the fields from `/component_template` as the Canonical Resource and inserts the captured identity at the configured Canonical identity pointer

#### Scenario: Direct response already has canonical shape
- **WHEN** a one-Resource response directly contains the configured identity and Resource fields
- **THEN** the system persists that response without adding a response or Resource Type wrapper

#### Scenario: Existing canonical identity conflicts with decoded identity
- **WHEN** decoded response identity differs from an identity already present in the decoded Resource body
- **THEN** the system reports a Transformation Conflict and does not persist the ambiguous Resource

### Requirement: Response collection shape is explicit
A many-Resource response mapping SHALL distinguish a list from an identity-keyed map. It SHALL independently describe the response pointer, the identity location within a list entry when needed, and the Resource location within an enveloped entry when needed. Direct list items and direct map values SHALL require no item Resource pointer.

#### Scenario: Decode a direct list
- **WHEN** the configured response pointer selects a list of direct Resource objects
- **THEN** each list element becomes one Resource and supplies identity through the Resource Type identity pointer

#### Scenario: Decode an identity-keyed map
- **WHEN** the configured response pointer selects a map from Resource ID to Resource object
- **THEN** each map key becomes that Resource's identity and each complete map value becomes its Resource body, including writable content and metadata siblings

#### Scenario: Decode enveloped list entries
- **WHEN** the configured response pointer selects a list whose entries contain separate identity and Resource subtrees
- **THEN** the system extracts both values and produces a Canonical Resource containing the Resource subtree plus its identity

#### Scenario: Collection has the wrong JSON shape
- **WHEN** a response mapping declares `list` but selects a JSON object, or declares `map` but selects a JSON array
- **THEN** the system reports a Transformation Conflict without persisting partial results

### Requirement: Mutation response semantics are explicit
An omitted mutation `response`, or an explicit `response: status`, SHALL determine success from the configured HTTP outcome without interpreting the response body as a Resource. For a one-Resource create, update, or upsert, `response: resource` SHALL treat the response body as the authoritative post-mutation Resource, and a response mapping object SHALL treat its decoded result as the authoritative post-mutation Resource. Many-Resource mutations and delete Operations SHALL use status response handling. The removed `trustworthy_response` field SHALL be rejected.

#### Scenario: Mutation defaults to status
- **WHEN** a successful create, update, or upsert Operation omits `response`
- **THEN** the system ignores the response body as Resource state and invalidates any prior observation

#### Scenario: Mutation returns a direct Resource
- **WHEN** a mutation declares `response: resource` and returns one Resource containing the configured identity
- **THEN** the system normalizes that body and uses it as authoritative post-mutation state

#### Scenario: Mutation returns a mapped Resource
- **WHEN** a mutation declares a response mapping object
- **THEN** the system applies the configured response mapping and uses the decoded Resource as authoritative post-mutation state

#### Scenario: Pending create needs server identity
- **WHEN** a target-scoped pending create relies on a server-assigned identity
- **THEN** it declares `response: resource` or a response mapping capable of yielding that identity

### Requirement: Single-resource mutation bodies are direct by default
A create, update, or upsert Operation with cardinality `one` SHALL use the Canonical Resource as its body without requiring an Operation Transformation. Before transmission, the system SHALL omit declared metadata and SHALL apply the Operation's identity body policy.

#### Scenario: Path-bound identity is omitted by default
- **WHEN** an Operation path contains `{id}` and the Operation does not override identity body policy
- **THEN** the system substitutes the Resource ID into the path and removes the configured identity pointer from the request body

#### Scenario: Identity defaults into a body without a path binding
- **WHEN** an Operation path does not contain `{id}` and the Operation does not override identity body policy
- **THEN** the request body retains the configured identity pointer

#### Scenario: API requires identity in both path and body
- **WHEN** an Operation whose path contains `{id}` explicitly enables identity in the body
- **THEN** the request path and body both contain the Resource identity

#### Scenario: API rejects identity without a path binding
- **WHEN** an Operation explicitly disables identity in the body
- **THEN** the request body omits the configured identity pointer regardless of the Operation path

#### Scenario: Metadata is tracked in Git
- **WHEN** a Canonical Resource retains opted-in metadata
- **THEN** a single-resource mutation omits every declared metadata field while retaining it in the Canonical Resource

#### Scenario: Writable content is one Canonical subtree
- **WHEN** an Operation declares `body: /policy` for a Canonical Resource containing identity, metadata, and a `/policy` subtree
- **THEN** the request body is the value at `/policy` after generic metadata and identity processing

#### Scenario: Body selector does not match
- **WHEN** an Operation body JSON Pointer does not select a value from the prepared Canonical Resource
- **THEN** payload construction reports a Transformation Conflict without sending a request

### Requirement: Many-resource bundling separates shape from format
A many-Resource mutation SHALL declare a collection shape independently from its payload serialization format. A `list` shape SHALL preserve Resource order and encode each prepared Resource as an item. A `map` shape SHALL use each Resource ID as a unique key and encode its prepared Resource as the corresponding value.

#### Scenario: JSON list bundle
- **WHEN** an Operation bundles multiple Resources with `shape: list` and JSON format
- **THEN** the request body is one JSON array containing the prepared Resources

#### Scenario: JSON map bundle
- **WHEN** an Operation bundles multiple Resources with `shape: map` and JSON format
- **THEN** the request body is one JSON object keyed by Resource ID, with identity omitted from each value by default because it is represented by the map key

#### Scenario: NDJSON list bundle
- **WHEN** an Operation bundles multiple Resources with `shape: list` and NDJSON format
- **THEN** the request body contains one prepared Resource per line in deterministic Resource order

#### Scenario: Unsupported map serialization
- **WHEN** a catalog combines `shape: map` with NDJSON or another format that cannot preserve one keyed collection
- **THEN** catalog validation rejects the Operation before any network request

#### Scenario: Duplicate map identity
- **WHEN** two Resources in a map bundle resolve to the same Resource ID
- **THEN** payload construction fails without sending a request

### Requirement: Envelope cleanup is not a general Transformation
Catalog validation SHALL reject the removed `frame` Transformation. Built-in catalogs SHALL express response envelopes through response mapping, express collection request envelopes through bundling, and use `body: <json-pointer>` when a one-Resource API accepts only a Canonical subtree. Ordinary one-Resource mutation Operations SHALL remain untransformed when the API accepts the full prepared Canonical Resource.

#### Scenario: Catalog uses legacy frame
- **WHEN** a catalog declares a Transformation with `kind: frame`
- **THEN** catalog validation rejects it as unsupported

#### Scenario: Component template mutation
- **WHEN** a normalized component template is created or updated
- **THEN** the API receives the component-template fields directly, without `name` or a `component_template` wrapper

### Requirement: Canonical shape changes require reconciliation
Changes to response mapping, collection shape, identity body policy, bundle shape, or metadata pointers SHALL participate in catalog and Observed State bindings. Existing wrapped Resources SHALL not be pushed with a newly selected definition until Fetch and Pull reconcile them to the new Canonical shape.

#### Scenario: Installed catalog changes canonical shape
- **WHEN** a repository has Observed State from a wrapper-preserving catalog and installs a normalized catalog definition
- **THEN** Push remains blocked until Fetch observes the normalized representation and Pull rewrites or conflicts with the Canonical Resource

#### Scenario: Pull normalizes an unchanged local Resource
- **WHEN** the local wrapped Resource still equals the prior Fetch baseline and the new Fetch observes the equivalent normalized Resource
- **THEN** Pull replaces the wrapped file with the normalized Canonical Resource while retaining its Resource ID and any tracked metadata
