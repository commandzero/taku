# Resource metadata specification

## Purpose

Defines how Application catalogs identify API-owned Resource metadata and how tracked, directory-local hint manifests let each Target and physical Resource Type directory choose whether that metadata enters Git without entering remote mutations.

## Requirements

### Requirement: Resource Types declare metadata explicitly
Each Resource Type Definition MAY contain a `metadata` object with a required non-empty `fields` list of exact JSON pointers. Metadata pointers SHALL address the Resource's Canonical Representation after inbound Resource Type transformations and before outbound Resource Type transformations. Application definitions SHALL classify metadata but SHALL NOT decide whether a repository directory tracks it. A Resource Type with no `metadata` object SHALL retain its existing transformation and reconciliation behavior.

#### Scenario: Resource Type declares API-owned fields
- **WHEN** a Resource Type Definition declares `metadata.fields` as `[/created_by, /updated_at]`
- **THEN** Taku classifies those two Canonical Representation fields as metadata for that Resource Type

#### Scenario: Resource Type has no metadata declaration
- **WHEN** a Resource Type Definition omits `metadata`
- **THEN** Taku applies no metadata-specific removal, comparison, or payload behavior to its Resources

#### Scenario: Declared field is absent
- **WHEN** a valid metadata pointer does not match a field in a particular Resource
- **THEN** metadata processing leaves that Resource unchanged and does not fail the operation

### Requirement: Metadata tracking hints are directory-local
Taku SHALL recognize optional, tracked `.target.yaml` and `.resource.yaml` hint manifests with closed, schema-versioned formats. `.target.yaml` SHALL be valid only at the root of a Target directory. `.resource.yaml` SHALL be valid only in a physical Resource Type directory whose direct children are Canonical Resource files or projected Resource directories. A hint SHALL derive Environment, Target, Namespace, Application, and Resource Type identity from its validated filesystem location and SHALL NOT repeat those identities in its content.

#### Scenario: Single-layout Target hint
- **WHEN** a Single Project contains `<target>/.target.yaml`
- **THEN** Taku binds the hint to that Project's one Environment and the Target named by the parent directory

#### Scenario: Multi-layout Target hint
- **WHEN** a Multi Project contains `<environment>/<target>/.target.yaml`
- **THEN** Taku binds the hint to the Environment and Target named by the two parent directories

#### Scenario: Non-namespaced Resource Type hint
- **WHEN** a Project contains `<target>/<resource-type>/.resource.yaml` in Single layout or `<environment>/<target>/<resource-type>/.resource.yaml` in Multi layout
- **THEN** Taku binds the hint to that non-namespaced physical Resource Type directory

#### Scenario: Namespaced Resource Type hint
- **WHEN** a Project contains `<target>/<namespace>/<resource-type>/.resource.yaml` in Single layout or `<environment>/<target>/<namespace>/<resource-type>/.resource.yaml` in Multi layout
- **THEN** Taku binds the hint only to that Namespace's physical Resource Type directory

#### Scenario: Hint does not duplicate hierarchy
- **WHEN** a valid hint is loaded
- **THEN** its content requires no Environment, Target, Namespace, Application, or Resource Type selector

### Requirement: Hint manifests expose only supported hints
Both hint manifests MAY contain `metadata.track: true | false`. Taku SHALL reject unknown fields and SHALL NOT interpret a hint manifest as a general merge or overlay of Project, Target, Application, or Resource Type configuration. In particular, hints SHALL NOT override identity, Operations, transformations, transport, namespacing, mutation, concurrency, dependency, projection, or Sensitive Field declarations.

#### Scenario: Target enables metadata tracking
- **WHEN** a Target directory's `.target.yaml` contains `metadata.track: true`
- **THEN** metadata tracking is enabled by that Target hint subject to a closer Resource Type hint

#### Scenario: Resource Type disables metadata tracking
- **WHEN** a Resource Type directory's `.resource.yaml` contains `metadata.track: false`
- **THEN** metadata tracking is disabled for Resources in that physical directory

#### Scenario: Hint attempts an Application override
- **WHEN** a hint contains an Operation path, transformation, or other unsupported configuration field
- **THEN** validation fails instead of merging the field into the installed Application definition

### Requirement: Closest directory hint determines tracking
For each Resource, an applicable `.resource.yaml` `metadata.track` value SHALL override the enclosing Target's `.target.yaml` value. If neither applicable manifest specifies a value, the effective value SHALL be `false`. The Target hint SHALL apply only to one Target instance, even when another Target uses the same Application. A namespaced `.resource.yaml` SHALL apply only within its own Namespace because each Namespace has a distinct physical Resource Type directory.

#### Scenario: No hint defaults to non-tracking
- **WHEN** a Resource Type declares metadata and neither applicable sidecar specifies `metadata.track`
- **THEN** Taku uses `track: false`

#### Scenario: Target opts all contained Resource Types in
- **WHEN** `.target.yaml` specifies `metadata.track: true` and a contained Resource Type directory has no override
- **THEN** Taku tracks declared metadata for Resources in that directory

#### Scenario: Resource Type opts out beneath Target
- **WHEN** `.target.yaml` specifies `metadata.track: true` and a contained `.resource.yaml` specifies `metadata.track: false`
- **THEN** Taku does not track declared metadata for Resources in that physical Resource Type directory

#### Scenario: Resource Type opts in without Target default
- **WHEN** `.target.yaml` is absent or specifies `metadata.track: false` and a contained `.resource.yaml` specifies `metadata.track: true`
- **THEN** Taku tracks declared metadata only for Resources in that physical Resource Type directory

#### Scenario: Targets using one Application differ
- **WHEN** two Target directories use the same installed Application but resolve different hints
- **THEN** each Target applies its own effective metadata tracking policy

#### Scenario: Namespaces differ
- **WHEN** two Namespaces contain the same Resource Type and only one physical Resource Type directory opts in
- **THEN** Taku tracks metadata only for Resources in the opted-in Namespace directory

### Requirement: Untracked metadata is excluded from desired state
For a Resource whose effective `metadata.track` value is `false`, Taku SHALL remove declared metadata from parsed remote Resources before comparing them with desired state or writing them through Add, Pull, or destination Promotion. Declared metadata SHALL therefore be absent from Canonical Resources and SHALL NOT create drift when only the remote metadata changes. Because declared metadata is API-owned, Taku SHALL also omit it from outbound payloads if it is manually present in an untracked Canonical Resource.

#### Scenario: Pull adopts a remote Resource
- **WHEN** Add or Pull materializes a remote Resource containing declared metadata in a directory where tracking is disabled
- **THEN** the resulting Canonical Resource omits every matching declared metadata field

#### Scenario: Only untracked metadata changes remotely
- **WHEN** Fetch observes a Resource whose managed content is unchanged and whose only remote changes are metadata not tracked by its directory
- **THEN** Status and Diff report no drift caused by those metadata changes

#### Scenario: Existing default behavior is preserved
- **WHEN** a former inbound `remove` transformation for an API-owned field is replaced by a metadata declaration and no applicable hint opts in
- **THEN** the field remains absent from fetched comparison values, Add output, Pull output, and Git-tracked Canonical Resources

#### Scenario: Untracked metadata is manually added
- **WHEN** a Canonical Resource contains declared metadata despite resolving `metadata.track: false`
- **THEN** Push excludes the metadata from equality and wire payloads rather than sending it to the remote API

### Requirement: Tracked metadata is retained without being sent
For a Resource whose effective `metadata.track` value is `true`, Taku SHALL retain declared metadata in parsed remote Resources, comparisons, Diff output, and Canonical Resources written by Add or Pull. Taku SHALL remove the declared metadata from every outbound create, update, or upsert payload, including bundled payloads, and SHALL exclude it from the equality check that decides whether Push needs a remote mutation.

#### Scenario: Pull tracks remote provenance
- **WHEN** Pull accepts a remote Resource containing metadata in a directory where tracking is enabled
- **THEN** the Canonical Resource contains that metadata so it can be reviewed and committed in Git

#### Scenario: Metadata-only remote change is pullable
- **WHEN** Fetch observes only a declared metadata change in a directory where tracking is enabled
- **THEN** Status and Diff expose drift and Pull updates the Canonical Resource with the remote metadata

#### Scenario: Managed content is pushed without metadata
- **WHEN** Push sends a create, update, upsert, or bundled mutation for a Canonical Resource containing tracked metadata
- **THEN** the wire representation omits every matching declared metadata field while retaining the other managed content

#### Scenario: Metadata-only local difference does not trigger Push
- **WHEN** the desired and observed Resources differ only at tracked metadata fields
- **THEN** Push reports the Resource as in sync and performs no remote mutation

#### Scenario: Locally authored Resource lacks metadata
- **WHEN** a new or edited Canonical Resource omits a metadata field in a directory where tracking is enabled
- **THEN** validation and Push remain valid and Taku does not synthesize a metadata value

### Requirement: Metadata composes deterministically with transformations and projections
Taku SHALL apply metadata processing at the Canonical Representation boundary: after all inbound Resource Type transformations and before reversing those transformations for outbound encoding. Existing explicit transformations SHALL continue in declared order, and Operation-specific outbound transformations SHALL receive a value from which all declared metadata has already been removed. `.resource.yaml` SHALL remain in the parent Resource Type directory and SHALL NOT become content of, or be replaced with, a projected Resource directory.

#### Scenario: Metadata follows inbound extraction
- **WHEN** an inbound Resource Type transformation extracts or restructures a response and a metadata pointer addresses the resulting Canonical Representation
- **THEN** Taku resolves the metadata pointer after that transformation

#### Scenario: Operation wraps an outbound body
- **WHEN** an Operation-specific transformation frames or bundles a Resource whose tracked metadata is present in Git
- **THEN** Taku removes the metadata before applying the Operation transformation and encoding the request

#### Scenario: Projected Resource is rewritten
- **WHEN** Pull atomically replaces a projected Resource directory beneath a Resource Type directory containing `.resource.yaml`
- **THEN** the hint remains untouched in the parent directory and is not included among projected Resource contents

#### Scenario: Non-metadata transformation remains directional
- **WHEN** a Resource Type retains explicit `remove` or `omit` transformations for fields not declared as metadata
- **THEN** those transformations preserve their existing inbound-only or outbound-only behavior

### Requirement: Hint files are recognized configuration inputs
Taku SHALL reserve the exact `.target.yaml` and `.resource.yaml` filenames within managed Resource trees and SHALL distinguish them from Canonical Resources, Deletion Markers, Namespace directories, and projected Resource content. Applicable hints SHALL participate in selected Git-state checks, Observed State bindings, and Push journal bindings. An unrelated hint outside the selected Resource scope SHALL NOT affect a scoped Push.

#### Scenario: Flat Resource scanner encounters Resource hint
- **WHEN** inventory scans a flat Resource Type directory containing `.resource.yaml`
- **THEN** Taku loads it as configuration and does not parse it as a YAML Resource

#### Scenario: Selected hint is uncommitted
- **WHEN** an applicable `.target.yaml` or `.resource.yaml` has selected uncommitted or untracked changes during Push
- **THEN** Taku applies the configured Push Git-State Policy to that hint just as it does to selected Resource inputs

#### Scenario: Unrelated hint is modified
- **WHEN** a scoped Push selects one Target or Resource Type and a hint outside that scope is modified
- **THEN** the unrelated hint does not block or require confirmation for that Push

#### Scenario: Effective hint changes after Fetch
- **WHEN** an applicable `metadata.track` value changes after Observed State was captured
- **THEN** that Observed State is structurally invalid and Push remains blocked until Fetch and Pull reconcile the new Canonical Representation

### Requirement: Resource lifecycle preserves directory hints
Add, Fetch, Pull, Remove, and Forget SHALL honor applicable hints but SHALL NOT create, rewrite, move independently, or delete them. Renaming a Target SHALL move its complete directory, including its `.target.yaml` and descendant `.resource.yaml` files. A Resource Type directory MAY remain as a configuration-bearing directory after its last Resource is removed or forgotten.

#### Scenario: Pull writes Resources beside a hint
- **WHEN** Pull changes Canonical Resources in a Resource Type directory containing `.resource.yaml`
- **THEN** the hint's bytes remain unchanged

#### Scenario: Last Resource is forgotten
- **WHEN** Forget removes the last Canonical Resource from a Resource Type directory containing `.resource.yaml`
- **THEN** the directory and its hint remain present

#### Scenario: Target is renamed
- **WHEN** Target Rename moves a Target directory containing hint manifests
- **THEN** the hints move with that directory and bind to the renamed Target path

### Requirement: Promotion uses destination-owned hints
Promotion SHALL NOT copy `.target.yaml` or `.resource.yaml` from the source. It SHALL resolve the destination Target and physical Resource Type directory hints and SHALL normalize promoted metadata according to the destination's effective tracking value before writing the destination Canonical Resource.

#### Scenario: Destination does not track source metadata
- **WHEN** a source Canonical Resource contains tracked metadata and the destination directory resolves `metadata.track: false`
- **THEN** Promotion omits that metadata from the destination Canonical Resource

#### Scenario: Source hints differ from destination hints
- **WHEN** source and destination directories have different hint manifests
- **THEN** Promotion leaves both manifests unchanged and applies only the destination hints

### Requirement: Invalid metadata or hint configuration fails before mutation
Catalog and filesystem validation SHALL reject an empty, duplicate, root, malformed, or overlapping metadata pointer; metadata that overlaps identity, display-name, catalog Sensitive Fields, or structural transformation state; an explicit `remove` or `omit` transformation that overlaps declared metadata; a tracked metadata pointer overlapping a Target-added Sensitive Field; and a hint with an unsupported schema version, unknown field, invalid value, symlink, invalid placement, unknown Target, or unavailable Resource Type. Validation failure SHALL occur before any network request or Project mutation.

#### Scenario: Metadata conflicts with identity
- **WHEN** a Resource Type declares its ID pointer or an ancestor of it as metadata
- **THEN** Application validation fails before the Application can be installed or used

#### Scenario: Tracked metadata conflicts with sensitive state
- **WHEN** an effective hint enables tracking for metadata that overlaps a Target-added Sensitive Field
- **THEN** validation fails rather than allowing a field configured for non-persistence to be tracked

#### Scenario: Metadata duplicates a directional transformation
- **WHEN** a metadata pointer overlaps a Resource Type `remove` or `omit` transformation
- **THEN** Application validation fails with an ambiguous metadata/transformation configuration

#### Scenario: Hint appears inside projected Resource
- **WHEN** a reserved hint filename appears inside an individual projected Resource directory
- **THEN** validation fails instead of treating it as projected content or a Resource Type hint

#### Scenario: Resource hint targets unavailable Type
- **WHEN** `.resource.yaml` is present in a Resource Type directory unavailable for the Target's selected Application Version
- **THEN** validation fails instead of silently ignoring the configuration input

#### Scenario: Hint is symlinked
- **WHEN** an applicable hint file or one of its path components is a symlink
- **THEN** validation fails before reading it as trusted configuration
