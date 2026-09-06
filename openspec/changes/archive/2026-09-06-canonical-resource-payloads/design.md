## Context

See [proposal.md](proposal.md) for motivation and [the capability spec](specs/canonical-resource-payloads/spec.md) for observable behavior.

The transport currently treats response extraction, collection expansion, per-item normalization, Resource Transformations, request-body framing, bundle serialization, and identity placement as loosely related steps. `expand_many` guesses list versus map from JSON shape, while `frame` is a bidirectional Transformation whose outbound behavior selects a Canonical subtree. Built-in component and index template definitions therefore preserve list-entry wrappers in Git and select their inner bodies again for PUT.

Resource identity is deliberately part of the Canonical Representation and cannot be inferred from a potentially lossy display-name filename. Metadata fields are also Canonical when tracking is enabled but remain API-owned. Both identity and metadata therefore need explicit, generic wire-body policies. Some APIs expose a complete per-Resource value containing metadata and a nested writable subtree; those Resources remain complete in Git and select the subtree explicitly at the mutation boundary.

## Goals / Non-Goals

**Goals:**

- Establish a pipeline with distinct response extraction, collection decoding, Canonical normalization, identity/metadata omission, collection bundling, and serialization stages.
- Make the common one-Resource mutation require no Operation Transformation.
- Preserve identity integrity when wire identity and Resource content are separated.
- Make list/map semantics explicit and independently testable from JSON/NDJSON/multipart encoding.
- Provide a safe Fetch/Pull migration for existing wrapped files.

**Non-Goals:**

- Removing genuine value Transformations such as embedded JSON, singleton-map conversion, insert, remove, extract, or omit.
- Inferring identity from filenames.
- Making metadata writable to remote APIs.
- Adding a general schema language for arbitrary response reshaping.

## Decisions

### 1. Add response mapping after Operation extraction

Add an optional closed `response` definition to `Operation`. It accepts scalar response semantics:

```yaml
response: status    # mutation default; classify by HTTP outcome and ignore the body
response: resource  # normalize the response body directly as one Resource
```

or a structured Resource mapping:

```yaml
response:
  collection: list       # optional: list | map
  identity_pointer: /name
  resource_pointer: /component_template
```

Read and list Operations consume Resource bodies by definition, so omission retains their direct-Resource behavior. Create, update, and upsert Operations default to `status`; they consume a response body only when configured as `resource` or with a mapping object. A mapping object implies a Resource response. The prior `trustworthy_response` boolean is removed because it described confidence rather than response semantics.

The existing `extract` and `unbundle` stages continue to select and parse the response payload. Response mapping then performs collection and item decomposition:

1. `collection: list` requires an array and visits elements in response order.
2. `collection: map` requires an object and treats each key as external identity while visiting entries in deterministic key order.
3. With no collection, the selected value is one response item.
4. `identity_pointer`, when present, captures identity from the response item before Resource extraction. A map key supplies identity without an identity pointer.
5. `resource_pointer`, when present, selects the Resource body from the response item. When absent, the item or map value is already the Resource body.
6. Resource Type inbound Transformations and metadata policy apply to that body.
7. Captured or requested identity is inserted at the reserved Canonical pointer `/_taku/id`. Existing wire fields at the configured identity pointer are retained; an existing unequal `/_taku/id` is a Transformation Conflict.

Many-cardinality JSON responses must declare `collection`. NDJSON unbundling is inherently a list stream but uses the same per-item pointer mapping and is modeled as `collection: list` for schema clarity. One-cardinality Operations may use response pointers without a collection.

This retains `extract_missing` behavior and avoids a larger replacement of response selection. It also separates response structure from Resource Type Transformations.

**Alternative considered:** Extend `extract` with list/map variants. Rejected because selecting a response subtree and interpreting the selected collection are orthogonal.

**Alternative considered:** Keep inferring arrays and maps. Rejected because a map may be one Resource object rather than a collection, and structural guessing hides catalog errors.

### 2. Keep identity Canonical and make body membership an Operation policy

Add `identity_in_body: Option<bool>` to mutation Operations. Its effective default is:

- `false` when the Operation path contains `{id}`;
- `true` otherwise;
- for a map bundle, `false` because the map key already carries identity.

An explicit value overrides the default, including the map default. Payload preparation captures the ID first, removes all declared metadata, applies identity omission to `/_taku/id` (and a matching configured API identity field when path-bound), and only then applies remaining Resource Type and Operation Transformations. When identity is required in the body and the API identity field is absent, the configured field is materialized from `/_taku/id`; an existing unequal wire field is retained rather than overwritten.

Identity insertion on reads and omission on writes are generic transport responsibilities. Built-in `omit` Transformations whose only purpose is stripping the configured identity are removed.

**Alternative considered:** Put body policy on `Identity`. Rejected because create/update endpoints for the same Resource Type can impose different body requirements.

**Alternative considered:** Always infer from `{id}` without an override. Rejected because some APIs require identity redundantly in both URL and body.

### 3. Make `body` either a static template or a Canonical subtree selector

Represent Operation `body` as a closed untagged choice:

- an object, array, number, boolean, or null remains the existing static request template;
- a JSON Pointer string such as `/policy` selects that subtree from each prepared Canonical Resource.

`body_pointer` remains valid only with a static body template, where it inserts prepared Resource content into an API-specific request envelope. A body selector runs after generic metadata and identity omission and after Resource Type value Transformations, replacing the current outbound use of `frame`. A missing selector is a Transformation Conflict.

For an ID-keyed response such as:

```json
{
  "daily": {
    "version": 1,
    "modified_date": "2026-01-01",
    "policy": { "phases": {} }
  }
}
```

map response decoding persists:

```json
{
  "_taku": { "id": "daily" },
  "version": 1,
  "modified_date": "2026-01-01",
  "policy": { "phases": {} }
}
```

The update Operation uses `body: /policy`. The Canonical Resource remains complete and metadata-aware, while PUT receives only the writable policy value.

**Alternative considered:** Copy selected response-envelope fields into an extracted Resource subtree. Rejected because the map value already is the complete single Resource; only the write API narrows it.

**Alternative considered:** Add another outbound Transformation name. Rejected because selecting the request body is an Operation concern and deserves direct vocabulary.

### 4. Add Collection Shape to Bundle independently of Payload Format

Extend `Bundle` with required `shape: list | map`:

```yaml
bundle:
  shape: list
  format: ndjson
  multipart: { ... }
```

`list` produces an ordered sequence. JSON serializes it as one array; NDJSON serializes one item per line. `map` captures each ID before body omission and produces one JSON object keyed by ID. Duplicate keys fail payload construction. `map` with NDJSON is invalid because it would not encode one keyed collection. Multipart remains an outer carrier for the chosen serialization and does not define collection shape.

Static Operation `body` plus `body_pointer` may receive either prepared single content or the fully shaped JSON collection. Catalog validation rejects combinations in which NDJSON cannot remain the top-level serialized stream.

**Alternative considered:** Use `array` and `object`. Rejected because `list` and `map` describe collection semantics, while an arbitrary Resource may itself be either JSON shape.

### 5. Remove `frame` rather than rename it

Remove `Transformation::Frame` from the accepted catalog schema. Response mapping eliminates collection-entry envelopes at ingestion, while `body: <json-pointer>` handles APIs that accept a narrower writable subtree. A renamed inverse Transformation would preserve the wrong seam.

Deserializing an old catalog containing `kind: frame` fails closed as an unsupported variant. The built-in catalog version is incremented so installed definitions and Observed State cannot silently mix shapes.

### 6. Normalize component/index template catalogs

Elasticsearch component and index template read/list Operations use response mappings that capture `/name` and extract `/component_template` or `/index_template`. Their Canonical Resources become:

```json
{
  "_taku": { "id": "esdiag@index" },
  "_meta": {},
  "template": {},
  "version": 2
}
```

Create/update paths bind `{id}`, so the reserved `/_taku/id` is removed and no Operation Transformation remains. Metadata pointers move from wrapper-relative paths such as `/component_template/created_date_millis` to Canonical paths such as `/created_date_millis`.

ILM and similar ID-keyed map responses retain the complete map value, receive `/_taku/id` as reserved canonical state, and use body selectors such as `/policy`. Other built-in `frame` and identity-only `omit` uses receive the same treatment according to their actual response shapes. Genuine request envelopes use a static `body` plus `body_pointer` or Bundle shape, not a response wrapper in Git.

### 7. Bind all shape decisions into reconciliation safety

Operation response mapping, identity body policy, Bundle shape, Resource Type identity, metadata pointers, and remaining Transformations are serialized within the installed catalog and existing observation binding material. Incrementing catalog versions plus the existing Baseline comparison makes the new Canonical definition structurally incompatible with prior observations.

After installation:

```text
old wrapped file + old observation
              │
              ▼ Fetch
old wrapped file + new flat observation  (Push blocked)
              │
              ▼ Pull
new flat file + new flat observation
```

Normal three-way Pull rewrites an unchanged old file. A locally modified wrapper conflicts rather than being destructively migrated.

### 8. Record the terminology and architectural boundary

Update the glossary so Canonical Representation explicitly means one Resource without response or collection envelopes. Add `Response Mapping` and `Collection Shape`. Record an ADR explaining why response decoding and request bundling are separate from Resource Transformations; the distinction is hard to reverse and otherwise surprising to future catalog authors.

## Risks / Trade-offs

- **[Breaking catalog schema]** Existing third-party catalogs using `frame` stop validating. → Fail closed with a precise error, document the response-mapping replacement, and bump built-in catalog versions.
- **[Existing repositories contain wrapped files]** A direct Push could send the wrong shape. → Catalog and observation binding changes block Push until Fetch and Pull reconcile the files.
- **[Identity appears in two wire locations]** Wrapper/key/requested identity can disagree with an inner body. → Compare all available identities and report a Transformation Conflict on disagreement.
- **[Default identity inference is occasionally wrong]** Some path-bound APIs also require body identity. → Provide the explicit per-Operation `identity_in_body` override.
- **[Collection configuration grows Operation schema]** More explicit catalogs are longer. → Keep direct items pointer-free and use one response mapping shared by read/list definitions where YAML flow style remains readable.
- **[Map ordering differs from remote order]** JSON object order is not semantically stable. → Canonically process maps in sorted key order; list is required when remote ordering is meaningful.

## Migration Plan

1. Add model types and strict validation while retaining current runtime behavior behind tests.
2. Implement response mapping, identity insertion/conflict checks, identity body policy, and shaped bundling.
3. Migrate every built-in catalog and remove all `frame` and identity-only `omit` uses.
4. Increment affected catalog versions and update metadata pointers.
5. Verify old wrapped fixtures cannot Push against the new binding, then Fetch/Pull them to the normalized form.
6. Update documentation, glossary, and ADR.

Rollback requires restoring the prior catalog and code together. Repositories already pulled into the normalized shape must Fetch and Pull with the restored catalog before Push; automatic reverse wrapping is intentionally not attempted.
