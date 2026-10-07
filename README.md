# Taku

Taku manages selected remote resources as plain-text, Git-versioned desired state. Git records what is accepted; Taku observes and compares it with remote state, adopts changes locally, and executes configured HTTP operations when you explicitly push.

The workspace contains the `taku` CLI and the reusable `resource-control` library. Elasticsearch and Kibana are the first embedded Application definitions; Projects may also use an explicitly refreshed Git Application Source.

## Install from source

Taku has **not yet been published** to crates.io, Homebrew, or a binary release. The repository is public; installation currently uses a source checkout. With Git on PATH and Rust 1.97.1 installed (minimum supported Rust: 1.89), run from this repository's root:

```sh
cargo install --path . --locked
```

Public distribution is still being prepared; see the [release policy](https://github.com/commandzero/taku/blob/main/docs/repository/releases.md) for current publication status.

## Quick start

From a Git worktree root, create a Project and register a Target. Replace the example URL with your own authorized endpoint:

```console
git init configuration
cd configuration
taku init --layout single --environment dev
taku install elasticsearch
taku target add elasticsearch es --url https://localhost:9200
taku list --remote --untracked es ingest_pipelines
taku add es ingest_pipelines pipeline-1
taku fetch es ingest_pipelines
taku status es
taku diff es ingest_pipelines pipeline-1
```

`install` vendors Application definitions locally; it does not install or change remote software. After reviewing the observed and desired Resource, reconcile deliberately:

```console
taku pull es ingest_pipelines pipeline-1 --yes
taku push es ingest_pipelines --dry-run
taku push es ingest_pipelines
```

| Command | Effect |
| --- | --- |
| `fetch` | Reads the Target and writes only ignored, local Observed State. |
| `status` / `diff` | Compare desired files with the last observation; do **not** contact the Target. |
| `pull` | Changes local desired files, never the remote Target; may report conflicts. |
| `push --dry-run` | Previews planned remote mutations without applying them. |
| `push` | **The only remote mutator**. Review the plan and Git state first. |

Omitting a Resource never deletes it remotely. `remove` creates a guarded Deletion Marker for a *later* Push; `forget` only stops local management. A Push is not atomic across APIs or reversible; inspect the Push Journal and reconcile before retrying after an interruption.

See [Using Taku](https://github.com/commandzero/taku/blob/main/docs/usage.md) for Resource Paths, Environments, namespaces, authentication, project files, output, completion, and the full safety model. For custom versioned catalogs, see [Application definitions](https://github.com/commandzero/taku/blob/main/docs/application-definitions.md). **Live validation can mutate services and retain resources**: read the [opt-in live API validation guide](https://github.com/commandzero/taku/blob/main/docs/live-api-validation.md) before running any live suite; never run the legacy `live_elastic` tests, which perform automatic DELETE cleanup.

## Development and license

See [contributing](https://github.com/commandzero/taku/blob/main/docs/repository/contributing.md) for setup and PR checks, the [documentation index](https://github.com/commandzero/taku/blob/main/docs/index.md) for more guidance, and the [release policy](https://github.com/commandzero/taku/blob/main/docs/repository/releases.md) for versions and publication.

Licensed under the [Apache License, Version 2.0](LICENCE.md).
