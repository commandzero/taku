## Why

Some catalog definitions preserve API collection envelopes such as `{ name, component_template: { ... } }` in Git, even though a single-resource create or update accepts only the inner Resource body. This makes the Canonical Representation mirror a list-response artifact and forces ordinary PUT/POST Operations to undo it with opaque `frame` Transformations.

## What Changes

- **BREAKING** Normalize every Canonical Resource to the single-resource representation, while retaining its Resource ID at the configured identity pointer for Git tracking.
- Add explicit response collection decoding for direct lists, ID-keyed maps, and enveloped list entries with separate wire identity and Resource pointers.
- Replace the confidence-based `trustworthy_response` flag with explicit response semantics: mutation responses default to HTTP status only, `response: resource` consumes a direct Resource body, and a response mapping consumes a structured Resource body.
- Make a one-Resource mutation body default to the Canonical Resource after generic omission of declared metadata and any identity already bound to `{id}` in the Operation path.
- Allow `body: <json-pointer>` to select the writable subtree of a Canonical Resource whose sibling fields include identity or API-owned metadata.
- Allow an Operation to override whether identity is included in its body for APIs that require or reject an exceptional shape.
- Extend many-Resource bundling with an explicit `list` or `map` shape independent of JSON, NDJSON, and multipart payload format.
- Remove the ambiguous `frame` Transformation and migrate built-in catalogs to response decoding, direct single-resource writes or explicit body selection, and collection bundling.
- Rebase metadata pointers and tests onto the normalized Canonical shape, with Fetch followed by Pull serving as the reconciliation path for existing repositories.

## Capabilities

### New Capabilities

- `canonical-resource-payloads`: Defines canonical single-resource shape, response collection decoding, identity placement in mutation bodies, and many-resource bundle shapes.

### Modified Capabilities

None.

## Impact

- Affects the catalog schema, Resource Type and Operation models, catalog validation, response normalization, outbound payload construction, built-in Elasticsearch/Kibana catalogs, observation bindings, metadata pointers, and transport/integration tests.
- Existing wrapped Canonical Resources become structurally stale after installing the new catalog version. Push remains blocked by catalog/Observed State bindings until Fetch and Pull rewrite them to the normalized shape.
- Catalog authors must replace `frame` and wrapper-preserving response definitions with explicit response and collection mappings.
