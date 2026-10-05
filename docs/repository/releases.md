---
type: Policy
title: Releases and compatibility
description: Package versions, compiler support, registry publication, and reviewed binary and Homebrew distribution.
generated: { by: openai-codex/gpt-6.1-sol, at: 2026-10-05T03:30:38Z }
---

# Releases and compatibility

Taku and resource-control use the Apache License, Version 2.0. The repository's authoritative [`LICENCE.md`](../../LICENCE.md) contains the complete terms; the separately redistributable engine crate includes its own identical license copy. Both packages inherit the `Apache-2.0` identifier. Binary archives include this text as `LICENSE` and the complete, source-backed dependency license texts and attribution in [`NOTICES.md`](../../NOTICES.md).

The first release proposal is 0.1.0; packages and binary assets have not been published. The CLI and reusable `resource-control` library release together from `workspace.package.version`, with an exact `=0.1.0` versioned path dependency in the CLI. Both manifests permit registry publication; that is preparation, **not authorization** to publish. The crates.io sparse index returned HTTP 404 for both names during preparation (2026-10-04); absence of indexed versions does not reserve ownership or guarantee publication. Recheck name ownership just before publishing. Keep the library separately consumable, and never weaken the CLI's exact dependency to work around registry propagation.

From the first published 0.x release, incompatible CLI, library, or persisted-format changes require a minor version increase and migration notes. Compatible fixes use patch releases. Preserve YAML as the default output and version persisted and output schemas explicitly. Review behavior, not just commit prefixes, when selecting a version.

Rust 1.89 is the minimum supported compiler. Development and release builds use pinned Rust 1.97.1. Verify the minimum with complete locked workspace tests and a fresh consumer of the published library. Compiler minimum increases belong in minor releases with changelog notes; verify selected dependency versions too.

Release helpers must support macOS's system Bash 3.2 and pass CI's ShellCheck as well as local preflight. Keep compound validation failures in explicit `if` branches to avoid `SC2015` diagnostics without suppressing guard checks.

Git is a runtime requirement for project discovery, history operations, and Git Application Sources. Generated Homebrew formulas declare `git` as a runtime dependency; binary-archive users must provide Git on PATH. README documentation links point to the repository so they also work from source packages that do not include `docs/`.

## Dependency license notices

Install cargo-about 0.8.4 with `rustup run 1.97.1 cargo install cargo-about --version 0.8.4 --locked`. Run `bash scripts/license-notices.sh` after changing dependencies, then review `NOTICES.md` and commit it with the lockfile. `bash scripts/license-notices.sh --check` regenerates into a temporary file without rewriting the committed notices. Code preflight and the shared archive packager require this check.

`about.toml` selects permissive terms already declared by the locked dependencies for default features across all three release targets; development-only dependencies are excluded and build dependencies retained conservatively. Preserve the complete license texts and copyright attribution. The generator refuses synthesized SPDX fallback text that lacks a source. Cargo-about's Chrono workaround separates its combined license file; hash-verified Ryu and sync_wrapper clarifications preserve their actual packaged Apache texts. Changed license files or new obligations require review, not a permissive fallback. See [cargo-about configuration](https://embarkstudios.github.io/cargo-about/cli/generate/config.html) and [source workarounds](https://embarkstudios.github.io/cargo-about/cli/generate/workarounds.html). Automated harvesting is not a legal compliance certification.


## Source package verification

Before the first registry publication, verify both interdependent source packages without uploading them:

```sh
rustup run 1.97.1 cargo package --workspace --registry crates-io --locked --allow-dirty
rustup run 1.97.1 cargo test \
  --manifest-path target/package/resource-control-0.1.0/Cargo.toml --locked
rustup run 1.97.1 cargo publish -p resource-control --dry-run --locked --allow-dirty
```

The dirty-worktree option is for preparation only; publish reviewed, clean source. The engine includes its embedded catalogs, license, and offline transport fixtures so its packaged unit tests remain runnable. The CLI excludes integration and live-test material; Cargo's ignored-test packaging warnings are intentional. Inspect the complete `.crate` inventories rather than assuming include patterns are root-anchored. Verify a fresh consumer of the extracted engine at the minimum compiler, with dependency resolution outside this workspace; registry resolution must be verified again after publishing the engine.


## First release: reviewed sequence

1. Obtain explicit approval to publish the crates.io packages and binary release assets. `commandzero/taku` is public, so source checkout is available, but public release download URLs and Homebrew installation remain unavailable until verified assets are published. Repository visibility is not package-publication authorization. Do not publish, push release tags, or open a tap PR merely to prepare the release.
2. Review a release PR with `workspace.package.version`, the exact CLI `resource-control` requirement, `Cargo.lock`, compatibility notes, and a dated `## [0.1.0] - YYYY-MM-DD` (or matching version) changelog section containing curated changes. Keep `Unreleased` ready for subsequent work. Run `bash scripts/preflight.sh`, `bash scripts/preflight.sh msrv`, and registry package checks on the reviewed source with Rust 1.97.1. Inspect `cargo package --list -p resource-control --allow-dirty` and `cargo package --list -p taku --allow-dirty`: the engine needs `assets/applications/` and `LICENCE.md`, the CLI source and license; no secret or private test material may enter either crate. `cargo package --workspace --registry crates-io --locked --allow-dirty` can generate dependent packages with both members selected before either exists in the registry; inspect the resulting `.crate` files and their normalized manifests/lockfiles. If the installed Cargo cannot perform joint workspace packaging, first publish the engine, wait for its exact version to resolve in crates.io, then `cargo publish -p taku --locked --dry-run` before publishing the CLI. Do not use `--no-verify` to claim a dry-run pass.
3. After approval and merge, create an annotated `v<version>` tag at the **reviewed, clean commit** (prereleases use the matching version/tag `v<version>-rc.1`), then run native packaging on each selected supported host. The tag-push `.github/workflows/release-prepare.yml` only creates ephemeral CI artifacts; it does not publish. Local and CI call the same script: `bash scripts/release-package.sh v0.1.0 <native-rust-target-triple> dist/v0.1.0/<native-rust-target-triple>`. Selected triples are `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`; each requires a native host, the pinned compiler and committed lockfile. A matrix entry is **not** a support claim until its archive has been built and tested on the intended host with recorded OS/distro/ABI floor. If one host is unavailable, do not claim or publish that target.
4. The script rejects a mistagged, dirty, or changelog-incomplete source, stale dependency notices, and existing artifact paths. It builds `taku-v<version>-<rust-target-triple>.tar.gz` with versionless `taku`, `LICENSE`, `NOTICES.md`, `BUILD-INFO.txt` at the archive root and a `<archive>.sha256` sidecar containing only the hash and basename. Build info records tag, commit, compiler, target, default features and builder; its OS/ABI floor explicitly remains uncertified until release notes document host evidence. It verifies checksum, extracts on the native host, checks version and byte-identical license/notice files, and performs offline `init`, embedded Application `install`, and `validate` in a temporary Git project. Archive checksum is integrity, not a signature or attestation. Run independent OS/ABI compatibility checks before making any broader promise.
5. Stage **all selected** verified archives/sidecars, retain the original bytes and hashes, and assemble a draft release with the curated changelog notes. Verify downloaded draft assets against retained checksums and archive provenance before making the release public; GitHub draft releases are not public Homebrew inputs. Do not use `--clobber`, replace published bytes, or treat a successful CI artifact upload as proof of a public download. Curate the target matrix, actual tested OS/ABI baselines and known unsupported platforms in release notes; flag prereleases.
6. With explicit publication authorization, publish the library first: `cargo publish -p resource-control --locked --dry-run`, then `cargo publish -p resource-control --locked`. Wait for `cargo info resource-control@0.1.0` to resolve from crates.io, verify the published source/embedded assets and a fresh consumer with Rust 1.89, and then run `cargo publish -p taku --locked --dry-run` followed by `cargo publish -p taku --locked`. Substitute the approved version everywhere. A local path may mask a missing exact registry dependency; do not infer registry availability from a workspace build. Publish the reviewed GitHub release assets only after all selected archives are verified and public visibility is settled; coordinate release order with registry and tap PR.
7. **Before promoting Homebrew**, generate and test a local formula from the real native archives and hashes in a disposable tap. Use a disposable prefix where available; otherwise first confirm that `taku` is not already installed and that the temporary tap name is unused. Preserve existing installations. Formula paths must be inside a tap's `Formula/` directory for Homebrew's formula-specific style rules. On macOS arm64:

   ```sh
   brew tap-new --no-git local/taku-release-check
   bash scripts/release-homebrew.sh v0.1.0 \
     dist/v0.1.0/aarch64-apple-darwin \
     "$(brew --repository local/taku-release-check)/Formula/taku.rb"
   brew style local/taku-release-check/taku
   HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 \
     brew install --formula local/taku-release-check/taku
   brew test local/taku-release-check/taku
   HOMEBREW_NO_AUTOREMOVE=1 brew uninstall local/taku-release-check/taku
   brew untap local/taku-release-check
   ```

   Local mode uses `file://` URLs and can generate a macOS-arm-only review formula from one tested archive; it is **not** a tap candidate and does not validate remote availability or Linux installation. Disable Homebrew autoremove during cleanup: it can otherwise remove pre-existing, unrelated packages.

   After publishing public assets and retaining all three verified target archives, run `bash scripts/release-homebrew.sh v0.1.0 --release /tmp/taku-public.rb`. This mode downloads all three public archives and their sidecars with HTTP failure checks, verifies checksum, archive layout, provenance, consistent commit and agreement with the local reviewed tag, and emits real GitHub URLs with computed SHA-256; a missing or private release fails. Review the generated formula against `commandzero/homebrew-tools` conventions, place it at `Formula/taku.rb` in the tap worktree, and verify actual install/version/offline `init`/`install`/`validate` on each advertised Homebrew host and public URL accessibility before proposing a tap PR. The tap's active formula must never reference an unavailable release. No Homebrew source fallback has been verified, so none is offered.

## Partial failure and immutable bytes

If the engine publishes but the CLI fails, retain that engine version, diagnose and retry the same reviewed CLI if its bytes are unchanged, or prepare a new version with exact dependency and changelog changes; published crates cannot be replaced. If GitHub upload partially fails, download and compare existing asset hashes to retained originals before uploading only missing assets; differing existing bytes require investigation and a new version, not force replacement. If the tap update fails after release, keep the existing formula unchanged and retry only the reviewed promotion against the same public release hashes. Coordinate any future tag, archive, checksum, license-entry, host URL, and formula URL migrations together; preserve historical tags and release assets.
