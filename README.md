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

For a Multi Project, initialize with `--layout multi --environments dev,stage,prod`, then select a default with `taku context set dev`. Resource commands accept repeatable `--environment`, `--target`, `--type`, and `--id` selectors; `--all-environments` is always explicit.

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
├── applications/<application>/resources.yml
└── baselines/<environment>/<target>.yml
```

Context, Application Source cache, Observed State, and Push Journals are ignored. In a Single layout, Resources use `target/type/name.json`; in a Multi layout they use `environment/target/type/name.json`. The stable Resource ID is stored in the payload, never inferred from the filename.

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

Ignored integration tests exercise complete Resource lifecycles against Elasticsearch at `http://localhost:9200` and Kibana at `http://localhost:5601`, plus Kibana's NDJSON Saved Objects export. They read `ELASTIC_API_KEY` from the repository's explicit `.env` file without mutating the test process environment. The value may be either the raw encoded key or an `ApiKey `-prefixed authorization value.

```console
cargo test --test live_elastic -- --ignored --nocapture
```

The lifecycle tests create uniquely named remote fixtures, manage them through the compiled `taku` CLI, and remove them through guarded Deletion Markers. A fallback cleanup guard also attempts deletion if an assertion fails. The export test is read-only. These tests remain skipped during ordinary `cargo test` runs.

## Application definitions

An Application is a strictly validated Target Profile and Resource Type Catalog. Each Resource Type declares identity, display-name policy, lifecycle Operations, actual HTTP methods, paths, headers and body templates, One/Many cardinality, independent request/response framing, transformations, write intent, retry safety, scheduling class, and dependencies. Many writes are bundled at runtime, including NDJSON payloads. HTTP verbs do not imply lifecycle semantics.

Update Mutation Mode is configured per Resource Type. Replace compares and owns the complete canonical document; Patch compares, pulls, and writes only the fields represented by the desired Resource. Target sensitive fields may tighten an Application's pre-persistence drops for a named Resource Type, but cannot remove identity or display-name state.

Git sources use this layout:

```text
applications/
└── custom-application/
    └── resources.yml
```

Configure `application_source.location` in `.taku/project.yml`, run `taku app refresh`, then install from the current offline cache. `taku update` is the only definition replacement workflow and refreshes each installed Git Application from its recorded source; `--from` explicitly switches eligible Applications to another source.

## Safety model

- Taku must run at a Git worktree root and never stages or commits files.
- Unknown configuration fields, invalid references, dependency cycles, unsafe paths, and ambiguous Variants fail before mutation.
- Symlinked or traversal-escaped Resource inputs are rejected.
- Omission always means unmanaged and never deletes a remote Resource.
- Push checks selected uncommitted and untracked inputs independently. Automation defaults both to `block` unless explicitly overridden.
- Serial Operations never overlap each other. Parallel Operations share the remaining capacity under `max_requests`.
- Confirmed Push outcomes are journaled durably. An identical retry skips successes; changed inputs cannot resume an incomplete plan.
- Failures block dependent Resource Types while independent work continues. Taku promises neither cross-API atomicity nor rollback.
