---
type: Guide
title: Contributing
description: Local validation, PR conventions, and OpenSpec completion checks.
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Contributing

Install the toolchain from `rust-toolchain.toml`, Rust 1.89.0 for minimum-compiler checks, ShellCheck, actionlint 1.7.12, OKF 0.2.7, and OpenSpec 1.11.0. OpenSpec is external authoring tooling; the application has no JavaScript build.

```sh
rustup toolchain install 1.97.1 --profile minimal --component rustfmt --component clippy
rustup toolchain install 1.89.0 --profile minimal
cargo install okf --version 0.2.7 --locked
cargo install tq-cli --version 0.3.0 --locked
go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
bun install --global @fission-ai/openspec@1.11.0
bash scripts/preflight.sh
bash scripts/preflight.sh msrv
```

Use `docs` mode for documentation-only changes and `code` for Rust and validation-tool changes. The default `all` runs both. Tests use local temporary projects and mock servers. The 11 live Elastic tests remain opt-in because they require external services and include remote mutations. Follow the root README when explicitly running them.

Keep the installed tools on PATH, including the Go and Bun global binary directories. CI uses the runner's npm to install the same pinned OpenSpec CLI.

CI runs Rust code checks in an unprivileged `pull_request` job with no repository secrets or shared dependency cache. A separate `pull_request_target` job runs documentation and OpenSpec contract checks from gate code copied from the trusted base revision. Pushes to `main` and manual runs execute the complete preflight from trusted repository contents.

Both packages inherit one version and compiler policy. Keep application builds locked. Run the minimum compiler separately; optional feature combinations need new checks if package features are introduced. The library remains reusable, but neither package publishes to a registry today.

Use Conventional Commit PR titles with `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, or `revert`. Add `!` and explain migration when the public contract breaks. Squash coherent changes into main; preserve historical commits. Add notable user effects to the changelog, keeping empty categories out.

## OpenSpec completion

Every PR body contains `OpenSpec: none` or `OpenSpec: change-id, another-id`. List changes implemented by the PR even when their artifacts were committed earlier. Reviewers must check the association and any no-delta explanation.

CI selects touched active and archived change paths from both sides of the merge-base diff, then adds explicitly associated IDs. Unrelated active changes do not block the PR. Selected changes must have one archive, completed tasks, and synchronized requirements and scenarios. Added and modified requirement blocks must match main specs after whitespace normalization. Removed requirements must be absent; renamed requirements must remove the old name and retain the new name. Put changed text for a renamed requirement under MODIFIED Requirements as well.

A change without spec deltas needs `no-spec-deltas.md` in its archive explaining why no contract changes. The checker confirms its presence; the reviewer confirms its reasoning. Validation alone does not constitute that review.

```sh
BASE_REF=origin/main OPENSPEC_CHANGES=my-change bash scripts/check-openspec.sh
```

With `BASE_REF`, the gate checks committed HEAD state against the merge base. Without it, local preflight checks working-tree changes against HEAD, including untracked files. `OPENSPEC_CHANGES` adds IDs in either mode. Set `CHECK_ALL_ARCHIVES=1` to check all archives against current specs, useful for this initial adoption but not a permanent gate once later changes supersede old requirements.

CI reruns on PR edits and synchronization, with no workflow path filters. After this adoption PR, require the `preflight`, `minimum compiler`, and `PR contract` results before merge when the GitHub plan permits branch protection. The adoption PR's bootstrap guard intentionally requires one manual preflight because its base revision cannot run a trusted workflow that does not exist there yet. The present private repository's API reports a plan restriction; until protection is available, maintainers must inspect those results before merging.

See [bundle rules](bundle.md) for documentation validation and [release policy](releases.md) for compatibility and distribution.

## Shared standards

The local shared bundle is `../../../repo-man/index.md` relative to this document's directory. Agents use the nearest workspace standards pointer and the repo-man skill when contributing. The repository adopts the audit remediation rules documented here; this does not change draft status in the shared standards bundle.
