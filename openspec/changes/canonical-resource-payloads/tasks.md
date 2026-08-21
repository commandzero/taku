## 1. Catalog Model and Validation

- [x] 1.1 Add closed `ResponseMapping` and `CollectionShape` model types with optional response identity and Resource pointers.
- [x] 1.2 Add the optional per-Operation `identity_in_body` override and make effective default behavior explicit in one helper.
- [x] 1.3 Add required `shape` to many-Resource `Bundle` definitions independently from payload `format`.
- [x] 1.4 Model Operation `body` as either an existing static template or a Canonical-subtree JSON Pointer selector.
- [x] 1.5 Remove `Transformation::Frame` from the accepted catalog model and every transformation match arm.
- [x] 1.6 Validate response mapping pointers, body selectors, cardinality/collection compatibility, map identity rules, Bundle shape/format compatibility, and static-body combinations before network access.
- [x] 1.7 Add model and catalog validation tests for valid direct/list/map/body definitions and every rejected schema combination, including legacy `frame`.
- [x] 1.8 Replace `trustworthy_response` with closed scalar `status`/`resource` response semantics while retaining the structured response mapping form.

## 2. Response Collection Decoding

- [x] 2.1 Replace structural guessing in many-response expansion with explicit direct, list, and map response decoding.
- [x] 2.2 Capture response-item or map-key identity before extracting an optional Resource subtree.
- [x] 2.3 Insert captured or requested identity at reserved `/_taku/id`, retain API identity fields, and reject disagreement with existing canonical identity.
- [x] 2.4 Preserve the intended ordering of response extraction, unbundling, guard capture, sensitive-field removal, Resource Transformations, and metadata tracking.
- [x] 2.5 Add transport tests for direct one-Resource responses, direct lists, ID-keyed maps, enveloped list entries, requested IDs, and deterministic map order.
- [x] 2.6 Add negative tests for wrong collection shape, missing pointers, non-string identity, conflicting inner/outer identity, and partial-result prevention.
- [x] 2.7 Default mutation responses to status-only handling and consume direct or mapped response bodies only when explicitly configured.

## 3. Direct Single-Resource Mutation Bodies

- [x] 3.1 Prepare one-Resource mutation bodies directly from the Canonical Resource after unconditional metadata omission.
- [x] 3.2 Omit reserved `/_taku/id` by default when the Operation path binds `{id}`, and materialize the configured API identity field only when body policy requires it.
- [x] 3.3 Honor explicit `identity_in_body: true` and `identity_in_body: false` overrides for create, update, and upsert Operations.
- [x] 3.4 Apply identity omission before remaining reversible Resource and Operation Transformations without changing genuine conversion behavior.
- [x] 3.5 Select an Operation `body` JSON Pointer after generic preparation and report a Transformation Conflict when it does not match.
- [x] 3.6 Add request-capture tests for default path-bound omission, default body identity, both overrides, nested identity pointers, tracked metadata, body selection, and direct untransformed PUT/POST bodies.

## 4. Explicit Many-Resource Bundling

- [x] 4.1 Carry each prepared Resource's ID alongside its body through grouped mutation scheduling and payload construction.
- [x] 4.2 Encode `shape: list` as a JSON array or deterministic NDJSON stream according to Bundle format.
- [x] 4.3 Encode `shape: map` as one JSON object keyed by Resource ID, with identity omitted from values by default.
- [x] 4.4 Reject duplicate map keys before sending a request and honor an explicit identity body override inside map values.
- [x] 4.5 Integrate list/map collections with supported static `body` and `body_pointer` envelopes and multipart carriage.
- [x] 4.6 Add unit and CLI tests for JSON list, JSON map, NDJSON list, multipart list, deterministic ordering, duplicate IDs, and unsupported map formats.

## 5. Built-in Catalog Normalization

- [x] 5.1 Audit every built-in response shape, `frame` Transformation, and identity-only `omit` Transformation against the new response and body policies.
- [x] 5.2 Normalize Elasticsearch component templates and index templates to flat Canonical Resources containing identity plus their single-resource API bodies.
- [x] 5.3 Migrate ILM, SLM, CCR auto-follow, enrich policy, and any other wrapper-preserving definitions without adding single-resource mutation Transformations.
- [x] 5.4 Migrate eligible Elasticsearch and Kibana identity-only `omit` definitions to generic Operation identity body policy while retaining genuine API-specific omissions.
- [x] 5.5 Rebase metadata, display-name, guard, dependency, and transformation pointers onto each normalized Canonical shape.
- [x] 5.6 Add explicit Bundle shapes to every many-Resource built-in mutation and explicit response collection shapes to every many-Resource read/list Operation.
- [x] 5.7 Increment affected built-in catalog versions and update catalog/model snapshot expectations.
- [x] 5.8 Remove redundant status-only mutation declarations and migrate authoritative mutation responses to `response: resource` or a mapping.

## 6. Reconciliation and Lifecycle Safety

- [x] 6.1 Verify response mapping, identity body policy, Bundle shape, and migrated pointers participate in catalog, Baseline, Observed State, and Push-plan bindings.
- [x] 6.2 Add an integration fixture with an old wrapped Canonical Resource and observation, then prove the new definition blocks Push until Fetch and Pull.
- [x] 6.3 Prove Pull rewrites an unchanged wrapped Resource to the normalized shape while preserving identity and opted-in metadata.
- [x] 6.4 Prove Pull reports a conflict rather than overwriting a locally modified wrapped Resource.
- [x] 6.5 Verify Add, Fetch, Status, Diff, Pull, Remove, Forget, Promotion, and Deletion Marker flows operate on normalized Resources.

## 7. Documentation and Domain Model

- [x] 7.1 Update catalog documentation and examples for response mapping, identity body policy, and Bundle shape, including migration from `frame`.
- [x] 7.2 Refine `Canonical Representation` in `CONTEXT.md` and add glossary entries for `Response Mapping` and `Collection Shape` without implementation detail.
- [x] 7.3 Add an ADR recording the separation of single-Resource Canonical shape, response collection decoding, and request collection bundling.
- [x] 7.4 Update metadata-tracking examples and documentation to use normalized Canonical metadata pointers.

## 8. Verification

- [x] 8.1 Format the workspace and run catalog/model/transport focused tests after migration.
- [x] 8.2 Run Clippy with warnings denied and the complete non-live test suite.
- [x] 8.3 Run live Elasticsearch component-template create/fetch/pull/push coverage using the normalized fixture.
- [x] 8.4 Strictly validate `canonical-resource-payloads` and the still-active `metadata-tracking` OpenSpec changes.
- [x] 8.5 Run focused and workspace verification for response semantics, formatting, Clippy, and strict OpenSpec validation.
