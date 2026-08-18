# Taku

Taku manages collections of remote Resources as version-controlled desired state. It is the reference CLI for the reusable resource-control engine, with Elastic Stack Cluster Administrators as its initial users.

## Language

**Project**:
A Taku-managed Git worktree whose root contains tracked project metadata and Resource Type Catalogs under `.taku/`. It uses one explicitly selected repository layout.
_Avoid_: Git repository, working directory

**Recognized Input**:
A schema-valid Project file that Taku identifies as selected configuration, an installed Application definition, a Resource, or a Deletion Marker. Git-state policies and remote operations ignore unrelated repository files.
_Avoid_: arbitrary repository file, glob match

**Directory Hint**:
A repository-owned policy attached to one physical Target or Resource Type directory, with its scope and identity supplied by that directory's location.
_Avoid_: Project overlay, Application default

**Resource Metadata**:
API-owned information in a Resource's Canonical Representation that may be retained for provenance but is never locally authoritative or writable.
_Avoid_: Managed Field, local metadata

**Repository Layout**:
The Project structure selected at initialization: Single omits an Environment directory and records its name in project metadata, while Multi begins each Resource tree with its Environment name.
_Avoid_: Overlay, auto-detected layout

**Application**:
An installable, reusable definition containing shared connection behavior and one Resource Type Catalog for each supported major Application Version. It is vendored into a Project and instantiated by Environment-specific Targets whose names default to the Application name.
_Avoid_: Target, remote application instance

**Application Version**:
The remote product version discovered from a Target and used to select a Major Version Catalog and its applicable Resource Type Definitions.
_Avoid_: Application Definition Version, schema version, catalog version

**Application Definition Version**:
The release identifier for the content of an Application Definition and its Resource Type Catalog.
_Avoid_: Application Version, schema version

**Application Installation**:
The definition-only vendoring of one or more Applications by `taku install`, without creating Targets. `taku target add` may perform the same installation atomically on first use after explicit confirmation or authorization.
_Avoid_: Target addition, remote installation

**Application Source**:
An embedded catalog or one Project-configured Git repository from which Applications may be installed or explicitly updated. An explicit `--from` source takes precedence over the configured source, which takes precedence over embedded Applications; this affects availability, not already vendored Applications.
_Avoid_: Application, package registry

**Application Source Cache**:
The disposable local result of explicitly refreshing a configured Git Application Source. Listing and installing Applications use the current cache without network access, while refresh and update may contact the source.
_Avoid_: Resource Type Catalog, installed Application

**Application Update**:
The atomic replacement of one or more vendored Applications by `taku update` after validating the complete selected set and its affected Resources. It preserves each Application's recorded source unless `--from` explicitly switches eligible Applications to another source.
_Avoid_: Resource Update, implicit catalog refresh

**Cluster Administrator**:
The person responsible for configuring one or more Elastic Stack clusters. Cluster Administrators are Taku's initial users.
_Avoid_: Operator, API developer

**Environment**:
A named deployment and Promotion boundary containing one or more Targets. Its desired configuration may vary from other Environments.
_Avoid_: Target, cluster

**Context**:
The local, non-authoritative selection of a current Environment and its provider settings.
_Avoid_: Environment, Target Baseline

**Command Scope**:
The Resources selected for one command. It defaults to every managed Resource in the current Environment and may be narrowed by the contiguous positional Resource Path `<target> <resource-type> <id>...`; crossing Environment boundaries is explicit, and any Resource Path resolves exactly one Environment.
_Avoid_: Resource Inventory, filesystem glob

**Context Provider**:
The Environment-level mechanism that resolves a Context and may supply shared connection metadata.
_Avoid_: Authentication Provider

**Target**:
A named remote application instance within an Environment that instantiates one installed Application's Target Profile and through which its assigned Resource Types are observed and changed. Its path name may default to the Application name, but remains explicit so one Environment can contain multiple instances of the same Application.
_Avoid_: Environment, Resource Type

**Target Addition**:
The creation of an Environment-specific Target by `taku target add <application> [name]`, defaulting its name to the Application. If the Application is absent, interactive confirmation or non-interactive `--yes` authorizes an atomic install-and-add using the current source cache.
_Avoid_: Resource Add, Application Installation

**Target Rename**:
The local, atomic renaming of a Target by `taku target rename <old> <new>` within one selected Environment, including its configuration and Resource tree, without changing Resource IDs or contacting the remote system. It invalidates Target-bound caches and incomplete Push Journals.
_Avoid_: Application rename, remote rename

**Target Profile**:
A reusable definition of non-secret application-wide transport defaults and Version Endpoints shared by corresponding Targets across Environments.
_Avoid_: Target, Environment

**Authentication Provider**:
The runtime mechanism that fully authenticates one Target. A Target-specific provider overrides an Environment default, and credential values are never desired state.
_Avoid_: Context Provider, credential file

**Secret Value**:
A resolved credential wrapped immediately at its provider boundary and excluded from serializable output models, diagnostics, journals, and caches. It is exposed only at the transport authentication boundary.
_Avoid_: redacted string, managed Resource field

**Sensitive Field**:
An exact structural field pointer whose value is dropped from a remote response before any Canonical Representation, cache, output, or other persistent form is written. Sensitive Fields cannot supply Resource identity or other required canonical state.
_Avoid_: Secret Value, masked persisted value

**Version Endpoint**:
One entry in an Application's ordered fallback list of remote requests and extraction rules for discovering its Application Version. Discovery succeeds at the first endpoint that returns an extractable valid version and fails only after every endpoint is exhausted.
_Avoid_: Fact Probe, health check, Resource Operation

**Target Baseline**:
The version-controlled record of the Application Version, Major Version Catalog, and Resource Type Definitions against which an Environment's Canonical Representations were last reconciled.
_Avoid_: Observed State Cache, version pin

**Resource Type**:
A category of remotely managed configuration bound to a Target Profile whose members share identity, lifecycle, and mutation semantics.
_Avoid_: API endpoint, adapter

**Namespace**:
An explicitly named isolation boundary within a Target that scopes Resources of Namespaced Resource Types. The default Namespace is explicit whenever namespacing is enabled.
_Avoid_: Environment, Target, implicit default

**Namespace Resource**:
An ordinary Resource whose lifecycle independently manages an application's Namespaces, such as a Kibana Space.
_Avoid_: Namespace directory, special command

**Namespaced Resource Type**:
A Resource Type that opts into Namespace-scoped identity and storage. Its Resources always include a Namespace directory, including `default`; Resource Types that do not opt in retain the non-namespaced layout.
_Avoid_: globally scoped Resource Type, optional path guessing

**Resource Type Dependency**:
An ordering relationship requiring all selected Resources of one Resource Type to succeed before a dependent Resource Type is changed.
_Avoid_: Resource reference, filesystem order

**Resource Type Catalog**:
The vendored, application-specific collection of Resource Type Definitions for one major Application Version.
_Avoid_: Resource inventory, payload manifest

**Major Version Catalog**:
The Resource Type Catalog selected by the major component of a Target's discovered Application Version.
_Avoid_: Application Definition, minor-version catalog

**Resource Type Definition**:
A complete, version-bounded declaration of one Resource Type's identity, lifecycle, transformations, and Operations. Its version constraint defaults to the enclosing Application Version constraint, and at most one definition for a Resource Type may apply to a Target.
_Avoid_: Resource Type Variant, configuration overlay

**API Stability**:
The declared lifecycle stage of a Resource Type Definition: Experimental, Preview, or Stable. An unclassified definition is Stable.
_Avoid_: Application Version, availability

**Operation**:
A named capability declared by a Resource Type for observing or changing Resources through a remote interaction. Its cardinality is One or Many.
_Avoid_: Resource, endpoint

**Operation Cardinality**:
The number of Resources an Operation handles in one remote interaction: One or Many.
_Avoid_: Single, bulk

**Outcome Mapping**:
The conversion of remote responses into Taku outcomes using conventional HTTP defaults with Resource Type overrides for exceptional APIs.
_Avoid_: Raw status code, error log

**Pagination Strategy**:
The bounded page-and-size or continuation-cursor rules through which a Many Operation obtains a complete set of uniquely identified Resources.
_Avoid_: Scripted loop, partial response

**Resource**:
A named unit of desired configuration belonging to a Resource Type.
_Avoid_: Object, request

**Pending Resource**:
A new Resource with a Target-scoped, server-assigned ID that has not yet been confirmed by Create. It cannot be promoted or used by identity-dependent workflows.
_Avoid_: Resource ID, temporary filename

**Resource ID**:
The stable, unique identity of a Resource within its Resource Type and declared scope. It may be simple or compound and is stored in the canonical Resource rather than inferred from its filename.
_Avoid_: Display Name, filename

**Resource ID Scope**:
The portability of a Resource ID: Universal IDs identify the same Resource across Targets, while Target IDs are meaningful only within one Target.
_Avoid_: Filename policy, Environment

**Display Name**:
A human-readable, potentially mutable label used to make a Resource easy to locate in the repository. It need not be unique unless its Resource Type declares and validates that invariant.
_Avoid_: Resource ID

**Canonical Representation**:
The deterministic, human-reviewable file or directory tree for one Resource stored in the repository. It may differ from the representation required by a remote system.
_Avoid_: Wire Representation, raw response

**Canonical Equality**:
Semantic equality between Canonical Representations after configured Transformations. Formatting and object-key order are irrelevant, while arrays and preserved text remain ordered unless a Resource Type explicitly normalizes them.
_Avoid_: Byte equality, wire equality

**Filesystem Projection**:
The reversible Resource Type mapping between one Resource Object and its Canonical Representation. Splitting projects the object to files, while Merging reconstructs the object from those files.
_Avoid_: Operation framing, arbitrary filesystem transform

**Resource Object**:
The structured representation of one Resource between Filesystem Projection and operation encoding.
_Avoid_: Resource collection, operation payload

**Splitting**:
The projection of one Resource Object into its configured Canonical Representation of one or more files.
_Avoid_: Unbundling, export

**Merging**:
The reconstruction of one Resource Object from its configured Canonical Representation of one or more files.
_Avoid_: Bundling, import

**Bundling**:
The encoding of a collection of Resource Objects into one operation payload, such as NDJSON.
_Avoid_: Merging, serialization

**Unbundling**:
The decoding of one operation payload, such as NDJSON, into a collection of Resource Objects.
_Avoid_: Splitting, deserialization

**Wire Representation**:
An API-specific encoding derived from one or more Canonical Representations at runtime, including multi-Resource encodings. It is not source-of-truth state.
_Avoid_: Canonical Representation, bundle artifact

**Transformation Conflict**:
A condition where Taku cannot safely convert a Resource between its Canonical and Wire Representations. It is reported without inserting conflict markers into the Resource.
_Avoid_: Parse warning, merge marker

**Pull Conflict**:
A condition where a Resource's working-tree and Observed State both changed differently from the local canonical value recorded at Fetch time. It is reported without modifying the Resource or inserting conflict markers.
_Avoid_: Git merge conflict, Transformation Conflict

**Presence Conflict**:
A condition where a managed Resource is present in the repository but conclusively absent from Observed State without a Deletion Marker explaining the difference.
_Avoid_: Not Found error, Deletion Marker

**Creation Conflict**:
A condition where Create may have succeeded but Taku did not receive a trustworthy Resource ID or confirmation. It blocks automatic retry and identity-dependent workflows until resolved.
_Avoid_: Pending Resource, retryable error

**Missing Policy**:
The Resource Type default or command-line override that determines whether a Presence Conflict remains unresolved or is explicitly resolved in the command's direction.
_Avoid_: Not Found handling, deletion policy

**Transformation**:
One schema-validated step in a Resource Type's ordered conversion between Canonical and Wire Representations.
_Avoid_: Script, arbitrary expression

**Retry Safety**:
The per-operation guarantee that determines whether Taku may retry after an uncertain or transient result. It is declared from endpoint behavior rather than inferred from the transport method.
_Avoid_: HTTP idempotency assumption

**Concurrency Mode**:
The Resource Type policy governing protection against writes based on stale Observed State. It defaults to Unguarded and may declare stronger operation-specific guards when the remote API supports them.
_Avoid_: Retry Safety, per-Resource override

**Concurrency Class**:
The Resource Type or Operation scheduling hint: Serial Operations never overlap one another, while Parallel Operations may share the remaining global request slots. Serial is the default.
_Avoid_: Concurrency Mode, request weight

**Embedded Document**:
Structured or textual content carried inside a Resource field. Strict JSON may be promoted to canonical structure, while YAML and other human-oriented formats remain readable multiline text when their comments and lexical form matter.
_Avoid_: Escaped blob

**Resource Inventory**:
The authoritative set of desired Resources intentionally managed by Taku for one Environment. It may represent only part of that Environment's remote Resources.
_Avoid_: Resource Type Catalog, payload manifest

**Observed State Cache**:
A disposable, non-authoritative snapshot produced by Fetch for later comparison or Pull and bound to the exact Project, configuration, Resource Type Catalog, Target, and Application Version used to create it. Deleting it never changes desired or remote state.
_Avoid_: State backend, desired state

**Observation Validity**:
The structural usability of Observed State based on its bound Project inputs, Application definitions, Target, Application Version, and Resource Type Definitions. Elapsed time is reported but does not itself invalidate an observation.
_Avoid_: cache TTL, concurrency guarantee

**Write Intent**:
The existence guarantee requested for a resource change: Create requires absence, Update requires existence, and Upsert permits either state. A resource type must reject an intent whose guarantee it cannot enforce.
_Avoid_: Apply

**Mutation Mode**:
The change semantics declared by a Resource Type for Update. Replace owns its complete manageable state; Patch owns only the specified state.
_Avoid_: Update mode

**Promotion**:
The deliberate transfer of a complete Canonical Representation with a Universal Resource ID between exactly corresponding Targets in two Environments, whether those Environments share a Multi-layout Project or occupy separate Single-layout Projects.
_Avoid_: Live transfer, environment copy

**Promotion Mapping**:
A destination-owned relationship in which a higher Environment names its default upstream Environment and each destination Target may name a differently named upstream Target. Unresolved selections are warned and skipped rather than matched by guesswork.
_Avoid_: Target-name equality, `to` mapping

**Deletion Marker**:
A temporary, Environment-specific instruction to delete a particular Resource only while its identity guard still matches. It is consumed after confirmed absence and may be forgotten without changing the remote Resource.
_Avoid_: Tombstone, Desired Absence

**Observed State**:
The normalized Resources currently retrieved from an Environment.
_Avoid_: Desired state, local state

**Fetch**:
The retrieval of Observed State for managed Resource IDs and Deletion Markers without changing desired Resources.
_Avoid_: Pull, sync

**Add**:
The adoption of selected remotely listed Resources into an Environment's Resource Inventory.
_Avoid_: Fetch, Git add

**Pull**:
The explicit acceptance of Observed State into the working tree as a proposed change to desired Resources.
_Avoid_: Fetch, Git pull

**Push**:
The only workflow that mutates remote application state, executing the validated plan represented by an Environment's Resource Inventory and Deletion Markers.
_Avoid_: Git push, direct API action

**Push Git-State Policy**:
The independent handling of uncommitted changes and untracked files selected by Push. Each condition may Block, require interactive Confirm, or Allow, with stricter effective defaults in non-interactive execution.
_Avoid_: clean-worktree requirement, Git mutation

**Push Journal**:
An ignored, durable record of an exact Push plan and its confirmed per-Resource outcomes. A matching retry resumes unfinished work without repeating successes; changed inputs invalidate the journal for resumption.
_Avoid_: desired state, rollback log

**Remove**:
The local replacement of a managed Resource with a Deletion Marker for a later Push.
_Avoid_: Immediate delete, Forget

**Forget**:
The local removal of a Resource or Deletion Marker from Taku management without changing remote state.
_Avoid_: Remove, Delete

**Status**:
A summary of whether effective desired state and Observed State agree and which operations are needed to reconcile them.
_Avoid_: Pull, detailed diff

**Diff**:
A detailed comparison of effective desired state and Observed State that changes neither.
_Avoid_: Status, Pull
