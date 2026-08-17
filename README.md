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
taku app add elasticsearch es --url https://localhost:9200
taku app add kibana kb --url https://localhost:5601
```

For a Multi Project, initialize with `--layout multi --environments dev,stage,prod`, then select a default with `taku context set dev`. Resource commands accept repeatable `--environment`, `--target`, `--namespace`, `--type`, and `--id` selectors; `--all-environments` is always explicit. A namespaced Resource Type requires an explicit namespace for remote listing, including `--namespace default`.

The normal observation and reconciliation loop is:

```console
taku list
taku list --remote --untracked --type ingest_pipelines
taku add --type ingest_pipelines --id pipeline-1
taku fetch
taku status
taku diff
taku pull --yes
taku push --dry-run
taku push
```

`fetch` writes only ignored Observed State. `status` and `diff` never contact a Target. `pull` changes only local desired files. `push` is the only remote mutator. `remove` creates a guarded Deletion Marker for a later Push; `forget` only stops local management.

All commands emit a schema-versioned YAML envelope by default. Use `--output json` for JSON. Differences exit successfully unless `status --check` or `diff --exit-code` is used.

## Project files

Tracked Taku metadata lives under `.taku/`:

```text
.taku/
├── project.yml
├── applications/<application>/
│   ├── application.yml
│   └── version-<major>.yml
└── baselines/<environment>/<target>.yml
```

Context, Application Source cache, Observed State, and Push Journals are ignored. In a Single layout, non-namespaced Resources normally use `target/type/name.json`; in a Multi layout they normally use `environment/target/type/name.json`. Resource Types that opt into namespacing add an explicit Namespace segment, including a literal `default` directory. A Resource Type may instead configure a Filesystem Projection, making one Resource a directory tree such as `target/namespace/skills/skill-id/SKILL.md`. The stable Resource ID remains in the merged Resource Object and is never inferred from the filesystem path. Namespace lifecycle is managed by an ordinary non-namespaced Resource Type defined by the Application, such as Kibana `spaces`.

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

## Live Elastic Stack tests

Ignored integration tests exercise complete Resource lifecycles against Elasticsearch at `http://localhost:9200` and Kibana at `http://localhost:5601`, plus Kibana's multipart-NDJSON Saved Objects import/export. They read `ELASTIC_API_KEY` from the repository's explicit `.env` file without mutating the test process environment. The value may be either the raw encoded key or an `ApiKey `-prefixed authorization value.

```console
cargo test --test live_elastic -- --ignored --nocapture
```

The lifecycle tests create uniquely named remote fixtures, manage them through the compiled `taku` CLI, and remove them through guarded Deletion Markers. A fallback cleanup guard also attempts deletion if an assertion fails. The export test is read-only. The full-corpus test uses the untracked sibling `taku-test-project` by default (or `TAKU_TEST_PROJECT`) and verifies all copied ESDiag component templates, index templates, pipelines, roles, spaces, saved objects, workflows, tools, and skills. These tests remain skipped during ordinary `cargo test` runs.

## Application definitions

An Application is a strictly validated `application.yml` plus one flat `version-<major>.yml` Resource Type Catalog per supported major product version. The application file declares shared transport defaults and an ordered fallback list of Version Endpoints. Taku tries each endpoint until one returns a valid Application Version, then selects the matching major catalog. Each Resource Type declares identity, optional namespacing, display-name policy, lifecycle Operations, actual HTTP methods, paths, headers and body templates, One/Many cardinality, independent Bundling and Unbundling, transformations, write intent, retry safety, scheduling class, and dependencies. Many writes are bundled at runtime, including multipart NDJSON payloads. HTTP verbs do not imply lifecycle semantics.

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

Each `version-<major>.yml` repeats the Application name, constrains its supported Application Versions, and defines each Resource Type as a list of complete version-qualified definitions:

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
        read: { method: GET, path: "/_ingest/pipeline/{id}", cardinality: one }
        upsert: { method: PUT, path: "/_ingest/pipeline/{id}", cardinality: one }
  future_resource:
    - version: ">=9.5.0, <10.0.0"
      stability: preview
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/future/{id}", cardinality: one }
        upsert: { method: PUT, path: "/future/{id}", cardinality: one }
```

An omitted Resource Type `version` inherits `application.version`; omitted `stability` defaults to `stable`. Zero matching definitions makes that Resource Type unavailable for the Target. More than one match is invalid configuration. Definitions are complete objects—Taku does not merge version overlays.

A display-name policy may define one `pointer` or an ordered `pointers` fallback list. The first pointer with a scalar value supplies the human-readable filename component; if none match, Taku falls back to the Resource ID. The `name_id` strategy appends up to the last eight characters of the Resource ID when that suffix is filename-safe, with an eight-character hash fallback for other IDs.

Filesystem Projection is separate from operation encoding. `split` converts one Resource Object into its canonical file tree; `merge` reconstructs it. `bundle.format` converts one or more Resource Objects into a request payload; an optional `bundle.multipart` configuration wraps that payload in a named multipart form part. `unbundle` decodes a response payload. Kibana Skills use the built-in `frontmatter_markdown` projection, while Kibana Saved Objects use NDJSON Unbundling and an explicitly configured multipart NDJSON Bundle.

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
```

For `frontmatter_markdown`, every unclaimed YAML frontmatter value passes through to the Resource Object. The Markdown body and referenced files are the only extracted fields. Splitting serializes passthrough values back to frontmatter; comments and original YAML formatting are not part of the API round trip.

Update Mutation Mode is configured per Resource Type. Replace compares and owns the complete canonical document; Patch compares, pulls, and writes only the fields represented by the desired Resource. Target sensitive fields may tighten an Application's pre-persistence drops for a named Resource Type, but cannot remove identity or display-name state.

Git sources use this layout:

```text
applications/
└── custom-application/
    ├── application.yml
    ├── version-8.yml
    └── version-9.yml
```

Configure `application_source.location` in `.taku/project.yml`, run `taku app refresh`, then install from the current offline cache. `taku update` is the only definition replacement workflow and refreshes each installed Git Application from its recorded source; `--from` explicitly switches eligible Applications to another source.

## Safety model

- Taku must run at a Git worktree root and never stages or commits files.
- Unknown configuration fields, invalid references, dependency cycles, unsafe paths, and overlapping Resource Type Definitions fail before mutation.
- Symlinked or traversal-escaped Resource inputs are rejected.
- Omission always means unmanaged and never deletes a remote Resource.
- Push checks selected uncommitted and untracked inputs independently. Automation defaults both to `block` unless explicitly overridden.
- Serial Operations never overlap each other. Parallel Operations share the remaining capacity under `max_requests`.
- Confirmed Push outcomes are journaled durably. An identical retry skips successes; changed inputs cannot resume an incomplete plan.
- Failures block dependent Resource Types while independent work continues. Taku promises neither cross-API atomicity nor rollback.
