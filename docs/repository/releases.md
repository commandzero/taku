---
type: Policy
title: Releases and compatibility
description: Package versions, compiler support, and the first binary release procedure.
generated: { by: codex/gpt-6, at: 2026-09-07T05:48:44Z }
---

# Releases and compatibility

Taku and resource-control use the Apache License, Version 2.0. The root `LICENSE` contains the complete terms; both packages inherit the `Apache-2.0` identifier from workspace metadata. Preserve applicable third-party license and attribution notices in distributions.

Taku is unreleased at 0.1.0. The CLI and resource-control library release together, using `workspace.package.version` as their version source. Keep the CLI's exact path-dependency version in agreement. Registry publication stays disabled until a separate distribution decision; a GitHub binary release does not require publishing either crate.

From the first published 0.x release, incompatible CLI, library, or persisted-format changes require a minor version increase and migration notes. Compatible fixes use patch releases. Preserve YAML as the default output and version persisted and output schemas explicitly. Review behavior, not just commit prefixes, when selecting a version.

Rust 1.89 is the minimum supported compiler. Development and release builds use the pinned toolchain. Verify the minimum with the complete locked workspace tests. Compiler minimum increases belong in minor releases with changelog notes; verify selected dependency versions too.

## Release procedure

1. Prepare a reviewed PR updating the shared version, exact CLI dependency, lockfile, and changelog together. Move Unreleased entries into a dated version section, add comparison links, and leave an Unreleased section for future work.
2. Run preflight and the minimum-compiler checks. Test on native macOS arm64, Linux x86_64, and Linux arm64 before claiming support for those targets. CI configuration is not proof that a platform passes. Record the runner OS and relevant minimum OS/ABI in the release notes.
3. Tag the reviewed commit `v<version>`, or `v<version>-rc.1` for a prerelease. Match the tag to the manifest and changelog. Build each target with the committed lockfile and pinned compiler.
4. Package `taku-v<version>-<rust-target-triple>.tar.gz`. Keep `taku`, `LICENSE`, applicable third-party license/notice files, and release build metadata at the archive root. Record tag, commit, compiler, target, default features, and OS/ABI limits. Each `.sha256` sidecar contains its hash and archive basename.
5. Extract every archive on its supported host. Verify checksums, `--version`, and an offline init/validate smoke test in a temporary Git project. Assemble a draft release only after all required checks pass and all selected target artifacts exist.
6. Publish the reviewed draft with curated changelog notes. Flag prereleases explicitly. Do not replace published bytes. Retry a partial upload only when existing asset hashes match; otherwise investigate and issue a new version.

There is no published installation channel or supported-platform claim yet. Wider distribution, registry publishing, and Homebrew updates need their own reviewed change. If registries are added, publish the engine before the CLI and verify its exact version resolves. A failed second publication must resume with the existing engine version or use a new version, never overwrite it.

Coordinate future tag, archive, checksum, and installer URL changes in one migration. Preserve historical tags and release assets.
