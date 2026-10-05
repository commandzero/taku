# Changelog

Notable user-facing changes are recorded here. The dated 0.1.0 section is the first release proposal; packages and binary assets have not been published yet.

## [Unreleased]

### Fixed

- Reject overlapping version-qualified Resource Type definitions and unavailable or cyclic dependencies before installing any selected Application, using the same validation for Elasticsearch, Kibana, and external Applications.
- Confine Target addition and rename to safe Environment/Target paths; reject symlinked Resource or cleanup paths and existing rename destinations before modifying desired state.
- Preserve Environment names requiring YAML quoting when saving Context.

## [0.1.0] - 2026-10-05

### Changed

- Simplify the README and move detailed usage, Application catalogs, and live-validation safety guidance into linked documentation.

### Fixed

- Exclude Kibana Skill `experimental` and Agent `created_by`/`type` response fields from create and update payloads, including when Resource metadata tracking is enabled.
- Return an I/O error instead of panicking when a data-report output pipe closes.
- Correct the domain glossary to describe parallel scheduling as the default.
- Declare Git as a runtime dependency in generated Homebrew formulas.
- Keep README documentation links usable from packaged CLI source.
- Require manual review for changes to dependency-notice validation code and inputs.
- Verify Homebrew archive licenses and notices against the reviewed tag, require matching provenance in local mode, and check the native binary version before formula generation.

### Added

- Prepare `taku` and `resource-control` source packages for crates.io and add clean-tag native archives with source-backed dependency license notices plus checksum-verified Homebrew formula generation. Publication remains a separately authorized operation.
- Add a configuration-driven live API validator for Taku/resource-control workflows, using declarative suites and real Application/Resource Type Catalogs rather than application-specific harness code. Require explicit mutation opt-in and externally supplied endpoint/auth environment variables; retain projects and reports without cleanup, with 60-second HTTP/CLI timeouts and strict fixture-defined assertions. Live success and complete API compatibility are not yet established; the saved-object absence diagnostic remains a known failure.
- Add offline catalog-driven transport fixtures with independent inbound and create/update/upsert expectations, checked with metadata tracking disabled and enabled.
- License Taku and resource-control under Apache-2.0.

- Manage selected remote Resources through explicit observation, comparison, adoption, promotion, and guarded Push workflows.
- Include Elasticsearch and Kibana Application definitions, with version-qualified Resource Type Catalogs.
- Support namespaced Resources, canonical payloads, directory-local metadata tracking, and filesystem projections.
- Provide positional Resource Paths and dynamic shell completion.
- Add contributor preflight, documentation validation, OpenSpec completion checks, and a shared package version and compiler-support contract.

[Unreleased]: https://github.com/commandzero/taku/commits/main
[0.1.0]: https://github.com/commandzero/taku/releases
