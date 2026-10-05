---
type: Guide
title: Using Taku
description: Resource workflows, project configuration, output, completion, and safety.
generated: { by: openai-codex/gpt-6-sol, at: 2026-10-05T01:20:56Z }
---

# Using Taku

From a Git worktree root, create a Project and register Targets:

```console
git init configuration
cd configuration
taku init --layout single --environment dev
taku install elasticsearch kibana
taku target add elasticsearch es --url https://localhost:9200
taku target add kibana kb --url https://localhost:5601
```

Replace the example URLs with your own authorized endpoints. Installation vendors Application definitions into the Project; it does not install software on the Targets. For a Multi Project, initialize with `--layout multi --environments dev,stage,prod`, then select a default with `taku context set dev`. `taku app` lists Applications; `taku target` lists the current Environment's Targets.

## Resource scope and reconciliation

Resource commands use one positional Resource Path: `<target> <resource-type> <id>...`. Local list/fetch/status/diff/pull/push accept any contiguous prefix, while add/remove/forget require the complete path and at least one ID. A supplied path resolves exactly one current or explicit `--environment`; broad commands without a path may repeat `--environment` or use `--all-environments`.

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

`list --remote` reads the remote inventory; `add` starts managing a selected Resource locally. `fetch` reads the Target and writes only ignored Observed State. `status` and `diff` compare the local desired files with that observation without contacting a Target. `pull` changes only local desired files and may conflict if both local and observed state changed. `push --dry-run` previews the plan; **`push` is the only remote mutator**. Review the plan and desired files before running it. `remove` creates a guarded Deletion Marker for a later Push; `forget` only stops local management. Omitting a Resource is not a remote deletion instruction.

Data commands emit a schema-versioned YAML envelope by default. Use `--output json` for JSON. Help, version, and completion scripts use their native text formats. Differences exit successfully unless `status --check` or `diff --exit-code` is used; conflicts always fail.

| Exit code | Meaning |
| --- | --- |
| 0 | Command completed; differences alone are successful unless checking was requested. |
| 2 | Invalid arguments, configuration, I/O, or another command error. Diagnostics go to stderr. |
| 3 | Status or Diff found differences with `--check` or `--exit-code`. |
| 4 | Structured conflict or unsuccessful Resource operation. Inspect the output envelope. |

Use `--non-interactive` for automation and supply required options explicitly. Taku reads Resource inputs from files; `-` is not a stdin alias. JSON and YAML reports contain one complete document and are assembled in memory. They are not streaming record protocols.

Network requests have a 30-second timeout. `max_requests` bounds concurrently scheduled requests, not total memory. Resource inventories, parsed documents, response bodies, and reports may remain in memory; there is no configurable input-size or nesting-depth budget. Use narrower Resource Paths for large inventories. Cancellation can interrupt execution after a remote operation succeeds. Inspect the Push Journal and reconcile before retrying; Taku does not promise rollback or a complete report after interruption. Output write failures return code 2; a partial output document is not valid confirmation of success.

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

Provider precedence is explicit `--set field=value`, process environment, then the configured dotenv file. Credential values never enter serializable reports, caches, Baselines, or Journals. Keep credential files outside tracked Project state.

## Shell completion

Taku generates dynamic completion for Bash, Zsh, Fish, Elvish, and PowerShell from the same Clap grammar used to parse commands:

```console
# Bash (replace `bash` with `zsh` for Zsh)
source <(taku completion bash)

# Fish
taku completion fish | source
```

Use `taku completion elvish` or `taku completion powershell` for the other supported shells. Completion offers command-specific Environments, Applications, Targets, Resource Types, Namespaces, IDs, provider keys, and filesystem paths. Local candidates only read Project state. Remote Type/ID candidates contact the one selected Target through bounded read operations but never refresh Application Sources or write Baselines, caches, desired Resources, or Project metadata. Dynamic lookup failures are silent so they do not disrupt the shell.

## Safety model

- Taku must run at a Git worktree root and never stages or commits files.
- Unknown configuration fields, invalid references, dependency cycles, unsafe paths, and overlapping Resource Type Definitions fail before mutation.
- Symlinked or traversal-escaped Resource inputs are rejected.
- Omission always means unmanaged and never deletes a remote Resource.
- Push checks selected uncommitted and untracked inputs independently. Automation defaults both to `block` unless explicitly overridden.
- Serial Operations never overlap each other. Parallel Operations share the remaining capacity under `max_requests`.
- Confirmed Push outcomes are journaled durably. An identical retry skips successes; changed inputs cannot resume an incomplete plan.
- Failures block dependent Resource Types while independent work continues. Taku promises neither cross-API atomicity nor rollback.
