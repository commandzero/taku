# Taku

Taku manages selected remote resources as plain-text, Git-versioned desired state. Git records what is accepted; Taku observes, compares, adopts, promotes, and executes configured HTTP operations.

The repository contains two Rust packages:

- `resource-control`: the reusable, application-neutral engine.
- `taku`: the single-binary reference CLI.

Elasticsearch and Kibana are embedded as the first Application definitions. Projects can also use an explicitly refreshed Git Application Source.

## Quick start

```console
git init configuration
cd configuration
taku init --layout single --environment dev
taku install elasticsearch kibana
taku target add elasticsearch es --url https://localhost:9200
taku target add kibana kb --url https://localhost:5601
```

For a Multi Project, initialize with `--layout multi --environments dev,stage,prod`, then select a default with `taku context set dev`. `taku app` lists Applications; `taku target` lists the current Environment's Targets. Resource commands use one positional Resource Path: `<target> <resource-type> <id>...`. Local list/fetch/status/diff/pull/push accept any contiguous prefix, while add/remove/forget require the complete path and at least one ID. A supplied path resolves exactly one current or explicit `--environment`; broad commands without a path may repeat `--environment` or use `--all-environments`.

Namespace remains the command-specific `--namespace` modifier. Exact IDs and remote listing require it for namespaced Types (including explicit `--namespace default`), non-namespaced Types reject it, and a local Type-only path without it spans all locally known Namespaces.

The normal observation and reconciliation loop is:

```console
taku list
taku list --remote --untracked es ingest_pipelines
taku add es ingest_pipelines pipeline-1
taku fetch es ingest_pipelines
taku status es
taku diff es ingest_pipelines pipeline-1
taku pull es ingest_pipelines pipeline-1 --yes
taku push es ingest_pipelines --dry-run
taku push es ingest_pipelines
```

`fetch` writes only ignored Observed State. `status` and `diff` never contact a Target. `pull` changes only local desired files. `push` is the only remote mutator. `remove` creates a guarded Deletion Marker for a later Push; `forget` only stops local management.

All commands emit a schema-versioned YAML envelope by default. Use `--output json` for JSON. Differences exit successfully unless `status --check` or `diff --exit-code` is used.

## Project files

Tracked Taku metadata lives under `.taku/`:

```text
.taku/
├── project.yaml
├── applications/<application>/
│   ├── application.yaml
│   └── version-<major>.yaml
└── baselines/<environment>/<target>.yaml
```

Context, Application Source cache, Observed State, and Push Journals are ignored. In a Single layout, non-namespaced Resources normally use `target/type/name.json`; in a Multi layout they normally use `environment/target/type/name.json`. Resource Types that opt into namespacing add an explicit Namespace segment, including a literal `default` directory. A Resource Type may instead configure a Filesystem Projection, making one Resource a directory tree such as `target/namespace/skills/skill-id/SKILL.md`. The stable Resource ID remains in the merged Resource Object and is never inferred from the filesystem path. Namespace lifecycle is managed by an ordinary non-namespaced Resource Type defined by the Application, such as Kibana `spaces`.

### Directory hints and Resource metadata

An Application catalog may classify API-owned Canonical fields that are useful for provenance but must never be sent back:

```yaml
metadata:
  fields: [/created_by, /updated_at]
```

Repositories opt into tracking those fields with closed, versioned Directory Hints. A Target default lives at its Target root:

```text
# Single                         # Multi
<target>/.target.yaml           <environment>/<target>/.target.yaml
```

A physical Resource Type override lives beside its Resources. Namespaced Types therefore have an independent override per Namespace:

```text
# Non-namespaced Single / Multi
<target>/<type>/.resource.yaml
<environment>/<target>/<type>/.resource.yaml

# Namespaced Single / Multi
<target>/<namespace>/<type>/.resource.yaml
<environment>/<target>/<namespace>/<type>/.resource.yaml
```

Both files have the same minimal shape and reject unknown fields:

```yaml
schema_version: 1
metadata:
  track: true
```

The closest value wins: `.resource.yaml`, then `.target.yaml`, then the default `false`. The path supplies all identity; hints contain no Environment, Target, Namespace, Application, or Resource Type selectors. For a projected Resource, `.resource.yaml` belongs in the parent Type directory, never inside an individual Resource directory.

With tracking disabled, Fetch removes declared metadata before comparison and Pull persistence. With tracking enabled, Status and Diff expose remote metadata drift and Pull writes it to the Canonical Resource. Push always removes declared metadata from equality and from create, update, upsert, and bundled wire payloads, so metadata remains remote-owned under either policy.

Hints are tracked configuration inputs: applicable files participate in Push Git-state checks and raw observation/journal bindings, while hints outside a scoped Push do not. Add, Fetch, Pull, Remove, and Forget never create, rewrite, or delete hints; forgetting the last Resource may leave a hint-only Type directory. Target Rename moves the complete directory. Promotion never copies source hints and normalizes the promoted Resource using only the destination hints.

Project metadata defines Environments and their named Targets:

```yaml
schema_version: 1
layout: multi
environments:
  dev:
    targets:
      es-dev:
        application: elasticsearch
        url: https://dev.example:9200
  prod:
    from: dev
    targets:
      es-prod:
        application: elasticsearch
        url: https://prod.example:9200
        from: es-dev
max_requests: 4
```

An authentication provider maps transport fields to environment-variable names. A Target provider completely overrides an Environment provider. One exact dotenv path may be configured; Taku parses it without discovering files or mutating the process environment.

```yaml
auth:
  dotenv: credentials.env
  fields:
    authorization: ELASTIC_AUTHORIZATION
```

Provider precedence is explicit `--set field=value`, process environment, then the configured dotenv file. Credential values never enter serializable reports, caches, Baselines, or Journals.

## Shell completion

Taku generates dynamic completion for Bash, Zsh, Fish, Elvish, and PowerShell from the same Clap grammar used to parse commands:

```console
# Bash (replace `bash` with `zsh` for Zsh)
source <(taku completion bash)

# Fish
taku completion fish | source
```

Use `taku completion elvish` or `taku completion powershell` for the other supported shells. Completion offers command-specific Environments, Applications, Targets, Resource Types, Namespaces, IDs, provider keys, and filesystem paths. Local candidates only read Project state. Remote Type/ID candidates contact the one selected Target through bounded read operations but never refresh Application Sources or write Baselines, caches, desired Resources, or Project metadata. Dynamic lookup failures are silent so they do not disrupt the shell.

## Live Elastic Stack tests

Ignored integration tests exercise complete Resource lifecycles against Elasticsearch at `http://localhost:9200` and Kibana at `http://localhost:5601`, plus Kibana's multipart-NDJSON Saved Objects import/export. They read `ELASTIC_API_KEY` from the repository's explicit `.env` file without mutating the test process environment. The value may be either the raw encoded key or an `ApiKey `-prefixed authorization value.

```console
cargo test --test live_elastic -- --ignored --nocapture
```

The lifecycle tests create uniquely named remote fixtures, manage them through the compiled `taku` CLI, and remove them through guarded Deletion Markers. A fallback cleanup guard also attempts deletion if an assertion fails. The export test is read-only. The full-corpus test uses the untracked sibling `taku-test-project` by default (or `TAKU_TEST_PROJECT`) and verifies all copied ESDiag component templates, index templates, pipelines, roles, spaces, saved objects, workflows, tools, and skills. These tests remain skipped during ordinary `cargo test` runs.

## Application definitions

An Application is a strictly validated `application.yaml` plus one flat `version-<major>.yaml` Resource Type Catalog per supported major product version. The application file declares shared transport defaults and an ordered fallback list of Version Endpoints. Taku tries each endpoint until one returns a valid Application Version, then selects the matching major catalog. Each Resource Type declares identity, optional namespacing, display-name policy, lifecycle Operations, actual HTTP methods, paths, headers, response mappings and body policies, One/Many cardinality, independent Bundling and Unbundling, transformations, write intent, retry safety, scheduling class, and dependencies. Many writes are bundled at runtime, including multipart NDJSON payloads. HTTP verbs do not imply lifecycle semantics.

Every configuration file carries its file-format `schema_version` and its own top-level definition `version`. The shared application file identifies the product and declares version discovery:

```yaml
schema_version: 1
version: "1.0.0"
application:
  name: elasticsearch
target_profile:
  headers:
    accept: application/json
version_endpoints:
  - method: GET
    path: /
    pointer: /version/number
```

Each `version-<major>.yaml` repeats the Application name, constrains its supported Application Versions, and defines each Resource Type as a list of complete version-qualified definitions:

```yaml
schema_version: 1
version: "1.2.0"
application:
  name: elasticsearch
  version: ">=9.0.0, <10.0.0"
resource_types:
  ingest_pipelines:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name_id }
      operations:
        read: { method: GET, path: "/_ingest/pipeline/{id}" }
        upsert: { method: PUT, path: "/_ingest/pipeline/{id}" }
  future_resource:
    - version: ">=9.5.0, <10.0.0"
      stability: preview
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/future/{id}" }
        upsert: { method: PUT, path: "/future/{id}" }
```

An omitted Resource Type `version` inherits `application.version`; omitted `stability` defaults to `stable`. Zero matching definitions makes that Resource Type unavailable for the Target. More than one match is invalid configuration. Definitions are complete objects—Taku does not merge version overlays.

`id.pointer` identifies an API field when a Resource carries its identity on the wire. Canonical Resources persist the stable ID separately at `/_taku/id`, so response mapping never has to overwrite an ordinary API field such as `name`.

Operations are retry-safe by default. Catalogs declare `retry_safe: false` only for exceptional endpoints that cannot safely repeat after a transient or uncertain result.

Operations run in parallel by default. Catalogs declare `concurrency: serial` only for expensive endpoints that need to be limited to one in-flight call.

Operations target one Resource by default. Catalogs declare `cardinality: many` only for Operations that read or write a collection.

Mutation Operations use HTTP status as their response by default and ignore the body as Resource state. Declare `response: resource` when the body is the authoritative Resource, or use a response mapping object when its collection, identity, or Resource envelope must be decoded. Read and list Operations consume Resource bodies by definition.

A display-name policy may define one `pointer` or an ordered `pointers` fallback list. The first pointer with a scalar value supplies the human-readable filename component; if none match, Taku falls back to the Resource ID. The `name_id` strategy appends up to the last eight characters of the Resource ID when that suffix is filename-safe, with an eight-character hash fallback for other IDs.

Filesystem Projection is separate from operation encoding. `split` converts one Resource Object into its canonical file tree; `merge` reconstructs it. A Many response must declare `response.collection` as `list` or `map`; `identity_pointer` captures an ID from a list entry, while an ID-keyed map supplies identity from its keys. `resource_pointer` then selects the Resource body inside each entry. This replaces structural guessing and keeps response collection wrappers out of the Canonical Representation.

For writes, `bundle.shape` independently declares a `list` or ID-keyed `map`, while `bundle.format` selects JSON or NDJSON encoding. An optional `bundle.multipart` wraps the encoded payload in a named form part. Map Bundles default to omitting identity from their values because the key carries it; List Bundles retain identity unless the Operation overrides `identity_in_body`. Kibana Skills use the built-in `frontmatter_markdown` projection, while Kibana Saved Objects use NDJSON Unbundling and an explicitly configured multipart NDJSON List Bundle.

```yaml
resource_types:
  skills:
    - version: ">=9.4.0, <10.0.0"
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: id, unique: true }
      filesystem:
        split: frontmatter_markdown
        merge: frontmatter_markdown
        frontmatter_markdown:
          document: SKILL.md
          body_pointer: /content
          referenced_files:
            pointer: /referenced_content
            path_pointer: /relativePath
            name_pointer: /name
            content_pointer: /content
            extension: md
      operations:
        list:
          method: GET
          path: /api/agent_builder/skills
          cardinality: many
          extract: /results
          response: { collection: list }
```

For `frontmatter_markdown`, every unclaimed YAML frontmatter value passes through to the Resource Object. The Markdown body and referenced files are the only extracted fields. Splitting serializes passthrough values back to frontmatter; comments and original YAML formatting are not part of the API round trip.

A Canonical Representation always describes one Resource and persists its ID at the reserved `/_taku/id` pointer even when the remote API puts that ID in a response-map key or request path. API identity fields remain ordinary wire fields and are never overwritten merely to persist Taku's identity. A path containing `{id}` omits `/_taku/id` and any matching API identity field from create, update, and upsert bodies by default; `identity_in_body` explicitly overrides that behavior. `body` may remain a static JSON request template, or a JSON Pointer string may select a narrower subtree after metadata and identity omission. For example, an ILM response keyed by policy name becomes one canonical object with `/_taku/id`, `version`, `modified_date`, and `policy` as siblings, while only `policy` is sent back:

```yaml
id: { pointer: /id, scope: universal }
metadata: { fields: [/version, /modified_date] }
operations:
  list:
    method: GET
    path: /_ilm/policy
    cardinality: many
    response: { collection: map }
  update:
    method: PUT
    path: /_ilm/policy/{id}
    body: /policy
```

For list entries such as `{name, component_template: {...}}`, `identity_pointer: /name` and `resource_pointer: /component_template` produce a flat canonical object containing the template fields plus `/_taku/id`; the response `name` is not copied into the Resource body. No create/update Transformation is required: the generic path-bound identity rule yields the API's direct single-template request body. Catalogs migrating from `kind: frame` should use Response Mapping for inbound collection envelopes and a `body` selector only when the write API genuinely accepts a narrower canonical subtree.

Update Mutation Mode is configured per Resource Type. Replace compares and owns the complete canonical document; Patch compares, pulls, and writes only the fields represented by the desired Resource. Target sensitive fields may tighten an Application's pre-persistence drops for a named Resource Type, but cannot remove identity or display-name state.

Git sources use this layout:

```text
applications/
└── custom-application/
    ├── application.yaml
    ├── version-8.yaml
    └── version-9.yaml
```

Configure `application_source.location` in `.taku/project.yaml`, run `taku app refresh`, then install from the current offline cache. `taku update` is the only definition replacement workflow and refreshes each installed Git Application from its recorded source; `--from` explicitly switches eligible Applications to another source.

## Safety model

- Taku must run at a Git worktree root and never stages or commits files.
- Unknown configuration fields, invalid references, dependency cycles, unsafe paths, and overlapping Resource Type Definitions fail before mutation.
- Symlinked or traversal-escaped Resource inputs are rejected.
- Omission always means unmanaged and never deletes a remote Resource.
- Push checks selected uncommitted and untracked inputs independently. Automation defaults both to `block` unless explicitly overridden.
- Serial Operations never overlap each other. Parallel Operations share the remaining capacity under `max_requests`.
- Confirmed Push outcomes are journaled durably. An identical retry skips successes; changed inputs cannot resume an incomplete plan.
- Failures block dependent Resource Types while independent work continues. Taku promises neither cross-API atomicity nor rollback.
