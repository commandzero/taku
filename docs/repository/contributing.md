---
type: Guide
title: Contributing
description: Local validation, PR conventions, and OpenSpec completion checks.
generated: { by: openai-codex/gpt-6.1-sol, at: 2026-10-07T04:14:11Z }
---

# Contributing

Install the toolchain from `rust-toolchain.toml`, Rust 1.89.0 for minimum-compiler checks, ShellCheck, actionlint 1.7.12, OKF 0.2.7, and OpenSpec 1.14.0. OpenSpec is external authoring tooling; the application has no JavaScript build.

```sh
rustup toolchain install 1.97.1 --profile minimal --component rustfmt --component clippy
rustup toolchain install 1.89.0 --profile minimal
cargo install okf --version 0.2.7 --locked
go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
bun install --global @fission-ai/openspec@1.14.0
bash scripts/preflight.sh
bash scripts/preflight.sh msrv
```

Use `docs` mode for documentation-only changes and `code` for Rust and validation-tool changes. The default `all` runs both. Ordinary tests use local temporary projects and mock servers; live validation is a separate, explicit opt-in, not evidence supplied by preflight. Follow the [recommended live API validator](../../README.md#configuration-driven-api-validation), not the legacy `live_elastic` tests: the latter perform automatic DELETE cleanup and must not be run. Do not use a blanket command to run all ignored tests.

Keep the installed tools on PATH, including the Go and Bun global binary directories. CI uses the runner's npm to install the same pinned OpenSpec CLI.

CI runs Rust code checks in an unprivileged `pull_request` job with no repository secrets or shared dependency cache. A separate `pull_request_target` job runs documentation and OpenSpec contract checks from gate code copied from the trusted base revision. Pushes to `main` and manual runs execute the complete preflight from trusted repository contents.

PR approval follows Agent + Copilot + Author review: the agent validates the implementation, Copilot reviews the current head with findings addressed, and the author decides whether to approve and merge. This includes workflow, toolchain, preflight and contract-helper changes. No independent human reviewer or non-author GitHub `APPROVED` review is required. Agents still need explicit authorization to merge into `main` or publish a release.

CI validates code and the documentation/OpenSpec contract; it does not turn review roles into a separate approval gate. No write token or repository secret is needed; `pull_request_target` has only contents/read permission and executes only base-snapshot helpers. An optional base `docs-index.sh` is copied when present so older trusted gates do not require newer PR-only helper inputs.

A CI policy correction cannot replace the workflow already executing from its base. Complete Agent + Copilot review and local preflight before the author approves landing that focused trusted-base update. Do not merge a release PR to bootstrap its own trusted workflow. After the update lands, synchronize dependent PRs with the trusted baseline and run checks on their new head/base.

Both packages inherit one version and compiler policy. Keep application builds locked. Run the minimum compiler separately; optional feature combinations need new checks if package features are introduced. The library remains reusable, but neither package publishes to a registry today.

Use Conventional Commit PR titles with `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, or `revert`. Add `!` and explain migration when the public contract breaks. Squash coherent changes into main; preserve historical commits. Add notable user effects to the changelog, keeping empty categories out.

## Configuration-driven validation

Keep API scenarios declarative. The generic [live harness](../../tests/live_api.rs) validates Taku/resource-control workflows through Application definitions, Resource Type Catalogs, direct HTTP observations, and the compiled CLI. Extend YAML fixtures rather than adding application-specific harness code; no legacy suite-format compatibility is required. The [live fixture guide](../../tests/fixtures/live/README.md) defines the current schema and safety contract.

The Elastic suite requires Elasticsearch/Kibana 9.4 with Agent Builder enabled. Externally supply `LIVE_ES_URL`, `LIVE_ES_AUTHORIZATION`, `LIVE_KB_URL`, and `LIVE_KB_AUTHORIZATION` in the process environment, never in secret files. A live invocation requires `TAKU_LIVE_SUITE`, an already-existing `TAKU_LIVE_ARTIFACTS` directory, and exactly `TAKU_LIVE_ALLOW_MUTATIONS=1`. Every HTTP request and CLI invocation is bounded to 60 seconds. Review fixtures and catalogs before granting access: the guardrails are not a security sandbox. Projects, reports, and remote resources remain after success or failure; there is no automatic cleanup.

Use the narrowly selected command in the root README. The `elastic.yaml` fixture covers pipeline create/update, a unique Kibana space, and skill/agent create/update. The separate `saved-objects.yaml` fixture is an honest known-failure diagnostic for fresh saved-object absence detection; preserve strict expectations rather than turning an unexpected response into a pass. Record the actual product versions, suite, and retained report when reporting results. Acquisition, create, update, and no-op coverage must be established by the authored steps and assertions; passing a suite is not complete API compatibility certification.

Run offline transport contracts without service credentials:

```sh
cargo test -p resource-control --locked catalog_fixtures -- --nocapture
```

The [offline suite](../../crates/resource-control/tests/fixtures/transport/suite.yaml) uses suite-relative catalogs and independent literal expectations for inbound data and create/update/upsert payloads, checking each declared write with metadata tracking both off and on. Set `TAKU_TRANSPORT_SUITE` to select an alternative suite. These checks do not establish live server compatibility.

When editing documentation in `docs/`, preserve existing metadata fields and update `generated.by` and `generated.at` to the actual editor and UTC edit timestamp. Follow the [bundle rules](bundle.md); regenerate indexes only when concept additions, renames, titles, or descriptions require it.

## OpenSpec completion

Every PR body contains `OpenSpec: none` or `OpenSpec: change-id, another-id`. List changes implemented by the PR even when their artifacts were committed earlier. Reviewers must check the association and any no-delta explanation.

CI selects touched active and archived change paths from both sides of the merge-base diff, then adds explicitly associated IDs. Unrelated active changes do not block the PR. Selected changes must have one archive, completed tasks, and synchronized requirements and scenarios. Added and modified requirement blocks must match main specs after whitespace normalization. Removed requirements must be absent; renamed requirements must remove the old name and retain the new name. Put changed text for a renamed requirement under MODIFIED Requirements as well. Main spec edits outside requirement blocks are currently unsupported; keep explanatory text stable while changing the requirement and scenario contract.

A change without spec deltas needs `no-spec-deltas.md` in its archive explaining why no contract changes. The checker confirms its presence; the reviewer confirms its reasoning. Validation alone does not constitute that review.

```sh
BASE_REF=origin/main OPENSPEC_CHANGES=my-change bash scripts/check-openspec.sh
```

With `BASE_REF`, the gate checks committed HEAD state against the merge base. Without it, local preflight checks working-tree changes against HEAD, including untracked files. `OPENSPEC_CHANGES` adds IDs in either mode. Set `CHECK_ALL_ARCHIVES=1` to check all archives against current specs, useful for this initial adoption but not a permanent gate once later changes supersede old requirements.

CI reruns on PR edits and synchronization, with no workflow path filters. Require the `preflight`, `minimum compiler`, and `PR contract` results before merge. The normal-PR bootstrap job checks that the base contains the actual contract-gate dependencies, not every new unprivileged validation input introduced by the PR. The author must inspect required results and Agent + Copilot review before approving a merge; repository branch protection is an additional control, not a replacement for validation.

See [bundle rules](bundle.md) for documentation validation and [release policy](releases.md) for compatibility and distribution.

## Shared standards

Agents use the repo-man skill and the repository policies documented here when contributing. The repository adopts the audit remediation rules documented here; this does not change draft status in any external standards bundle.
