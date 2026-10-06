# Changelog

Notable user-facing changes are recorded here. Taku has no published release yet.

## [Unreleased]

### Fixed

- Exclude Kibana Skill `experimental` and Agent `created_by`/`type` response fields from create and update payloads, including when Resource metadata tracking is enabled.
- Return an I/O error instead of panicking when a data-report output pipe closes.
- Correct the domain glossary to describe parallel scheduling as the default.
- Allow reviewed CI and validation-gate changes through exact-head human maintainer approval instead of an unconditional workflow-integrity failure, while retaining trusted-base execution and read-only credentials.
- Generate and validate numbered documentation indexes consistently with the adopted repository format while keeping trusted-base checks read-only.

### Added

- Add a configuration-driven live API validator for Taku/resource-control workflows, using declarative suites and real Application/Resource Type Catalogs rather than application-specific harness code. Require explicit mutation opt-in and externally supplied endpoint/auth environment variables; retain projects and reports without cleanup, with 60-second HTTP/CLI timeouts and strict fixture-defined assertions. Live success and complete API compatibility are not yet established; the saved-object absence diagnostic remains a known failure.
- Add offline catalog-driven transport fixtures with independent inbound and create/update/upsert expectations, checked with metadata tracking disabled and enabled.
- License Taku and resource-control under Apache-2.0.

- Manage selected remote Resources through explicit observation, comparison, adoption, promotion, and guarded Push workflows.
- Include Elasticsearch and Kibana Application definitions, with version-qualified Resource Type Catalogs.
- Support namespaced Resources, canonical payloads, directory-local metadata tracking, and filesystem projections.
- Provide positional Resource Paths and dynamic shell completion.
- Add contributor preflight, documentation validation, OpenSpec completion checks, and a shared package version and compiler-support contract.

[Unreleased]: https://github.com/commandzero/taku/commits/main
