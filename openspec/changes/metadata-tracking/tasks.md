## 1. Catalog and Hint Models

- [x] 1.1 Add the optional Resource Type `metadata.fields` declaration with serialization defaults that preserve existing Application catalogs.
- [x] 1.2 Add closed, schema-versioned `.target.yaml` and `.resource.yaml` models supporting only `metadata.track`, with unknown-field and invalid-value rejection.
- [x] 1.3 Add model tests for omitted metadata, empty/invalid field declarations, true/false hints, unsupported schema versions, unknown fields, and serialization round trips.
- [x] 1.4 Keep `.taku/project.yaml` unchanged and add tests proving Application definitions cannot set repository metadata tracking policy.

## 2. Directory Hint Resolution and Discovery

- [x] 2.1 Add canonical Target-root and physical Resource Type-directory path helpers covering Single/Multi, namespaced/non-namespaced, and projected/flat layouts.
- [x] 2.2 Add one hint resolver that reads only the exact expected `.target.yaml` and `.resource.yaml` paths and resolves Resource Type value over Target value over false.
- [x] 2.3 Reserve both filenames beneath managed Target trees and distinguish `.resource.yaml` from flat Resources, Deletion Markers, Namespace directories, and projected Resource contents.
- [x] 2.4 Extend recognized-input discovery so validly placed hint-only directories are retained and hints for unavailable Resource Types are reported rather than silently ignored.
- [x] 2.5 Add resolution tests for Target-instance independence, Resource Type opt-in/out, Namespace-local overrides, hint-only directories, and identical Applications with different directory policies.

## 3. Hint and Metadata Validation

- [x] 3.1 Validate hint placement against known Environment, Target, Namespace, selected Resource Type definition, and namespacing rules in both repository layouts.
- [x] 3.2 Reject reserved hint filenames at invalid levels, including inside individual projected Resource directories, before Resource or projection parsing.
- [x] 3.3 Reject symlinked hint files and symlinked path components using the existing managed-input safety rules.
- [x] 3.4 Validate catalog metadata pointers as unique, non-root Canonical JSON pointers with no duplicate or ancestor/descendant overlap.
- [x] 3.5 Reject metadata overlap with identity, display-name, catalog Sensitive Fields, structural/projection state, and explicit Resource Type `remove` or `omit` transformations.
- [x] 3.6 Reject effective tracked metadata that overlaps Target-added Sensitive Fields, while preserving Sensitive Field removal when tracking is disabled.
- [x] 3.7 Add negative integration tests proving every invalid hint or metadata configuration fails before network or filesystem mutation.

## 4. Git-State and Safety Binding

- [x] 4.1 Define stable hint binding material containing each expected path, presence/absence, and raw bytes for the applicable Target and physical Resource Type directory.
- [x] 4.2 Include hint binding material in Observed State structural bindings and Push plan/journal bindings for live and Baseline-based resolution.
- [x] 4.3 Extend selected Push Git-state paths with applicable Target and Resource Type hints for Resources and Deletion Markers.
- [x] 4.4 Test that applicable uncommitted/untracked hints follow Push Git-State Policy, unrelated hints do not affect scoped Push, and creating, editing, or deleting a hint after Fetch invalidates observation safety.

## 5. Directional Metadata Processing

- [x] 5.1 Add metadata-aware Canonical helpers that apply catalog transformations first, remove metadata inbound when tracking is false, and omit all declared metadata outbound regardless of tracking.
- [x] 5.2 Integrate hint-aware inbound processing into exact reads, lists, pagination, unbundling, Add, Fetch, comparison, and Pull persistence.
- [x] 5.3 Integrate unconditional metadata omission into single and bundled create, update, and upsert payload construction before Operation transformations.
- [x] 5.4 Normalize declared metadata out of desired and observed values before Replace/Patch Push equality so metadata-only differences never schedule a mutation.
- [x] 5.5 Preserve tracked metadata in Status, Diff, Add, and Pull while ensuring untracked or manually added metadata is never sent.
- [x] 5.6 Add transport and reconciliation tests for missing/nested pointers, structural extraction, Operation body selection, bundles, metadata-only drift, mixed changes, Pull conflicts, and manually added untracked metadata.

## 6. Resource Lifecycle, Projections, and Promotion

- [x] 6.1 Ensure Add, Fetch, Pull, Remove, and Forget never create, rewrite, or delete hint manifests and preserve a Resource Type directory containing only `.resource.yaml`.
- [x] 6.2 Verify projected merge/split excludes the parent `.resource.yaml`, rejects reserved names inside projected Resources, and preserves hints during atomic Pull replacement.
- [x] 6.3 Verify Target Rename moves `.target.yaml` and descendant `.resource.yaml` files with the complete Target directory and rebinds them by path.
- [x] 6.4 Make Promotion ignore source hints, resolve destination hints, and normalize promoted metadata before writing the destination Canonical Resource.
- [x] 6.5 Add lifecycle tests for last-Resource Forget, Deletion Markers, Target Rename, source/destination policy differences, and manual whole-directory portability.

## 7. Embedded Catalog Migration

- [x] 7.1 Audit Elasticsearch and Kibana Resource Type `remove` transformations and classify API-owned metadata separately from structural normalization.
- [x] 7.2 Replace classified Elasticsearch metadata removals with `metadata.fields` declarations and prove byte-equivalent Canonical and wire results without sidecars.
- [x] 7.3 Replace classified Kibana metadata removals with `metadata.fields` declarations and prove byte-equivalent Canonical and wire results without sidecars.
- [x] 7.4 Increment affected embedded catalog definition versions and verify Application updates invalidate stale observations.
- [x] 7.5 Update live Elastic assertions for migrated default behavior and add isolated Target-level and Resource Type-level opt-in lifecycles where suitable metadata exists.

## 8. Documentation and Verification

- [x] 8.1 Document `.target.yaml` and `.resource.yaml` placement for every layout, their minimal schemas, physical scope, precedence, default false, and closed-hint constraint in `README.md`.
- [x] 8.2 Document projection placement, recognized-input/Git safety, lifecycle preservation, destination-owned Promotion, and Status-versus-Push semantics.
- [x] 8.3 Add Directory Hint and Resource Metadata terminology to `CONTEXT.md` and record the filesystem-local configuration decision in an ADR.
- [x] 8.4 Run formatting, Clippy, the full non-ignored test suite, focused ignored live tests, and strict OpenSpec validation; fix all failures.
