---
type: Guide
title: Application definitions
description: Versioned catalogs, operation mappings, projections, and Git sources.
generated: { by: openai-codex/gpt-6-sol, at: 2026-10-05T01:20:56Z }
---

# Application definitions

An Application is a strictly validated `application.yaml` plus one flat `version-<major>.yaml` Resource Type Catalog per supported major product version. The application file declares shared transport defaults and an ordered fallback list of Version Endpoints. Taku tries each endpoint until one returns a valid Application Version, then selects the matching major catalog. Each Resource Type declares identity, optional namespacing, display-name policy, lifecycle Operations, actual HTTP methods, paths, headers, response mappings and body policies, One/Many cardinality, independent Bundling and Unbundling, transformations, write intent, retry safety, scheduling class, and dependencies. Many writes are bundled at runtime, including multipart NDJSON payloads. HTTP verbs do not imply lifecycle semantics.

Every configuration file carries its file-format `schema_version` and its own top-level definition `version`. The shared application file identifies the product and declares version discovery:

```yaml
schema_version: 1
version: "1.0.0"
application:
  name: elasticsearch
target_profile:
  headers:
    accept: application/json
version_endpoints:
  - method: GET
    path: /
    pointer: /version/number
```

Each `version-<major>.yaml` repeats the Application name, constrains its supported Application Versions, and defines each Resource Type as a list of complete version-qualified definitions:

```yaml
schema_version: 1
version: "1.2.0"
application:
  name: elasticsearch
  version: ">=9.0.0, <10.0.0"
resource_types:
  ingest_pipelines:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name_id }
      operations:
        read: { method: GET, path: "/_ingest/pipeline/{id}" }
        upsert: { method: PUT, path: "/_ingest/pipeline/{id}" }
  future_resource:
    - version: ">=9.5.0, <10.0.0"
      stability: preview
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/future/{id}" }
        upsert: { method: PUT, path: "/future/{id}" }
```

An omitted Resource Type `version` inherits `application.version`; omitted `stability` defaults to `stable`. Zero matching definitions makes that Resource Type unavailable for the Target. More than one match is invalid configuration. Definitions are complete objects—Taku does not merge version overlays.

`id.pointer` identifies an API field when a Resource carries its identity on the wire. Canonical Resources persist the stable ID separately at `/_taku/id`, so response mapping never has to overwrite an ordinary API field such as `name`.

Operations are retry-safe by default. Catalogs declare `retry_safe: false` only for exceptional endpoints that cannot safely repeat after a transient or uncertain result.

Operations run in parallel by default. Catalogs declare `concurrency: serial` only for expensive endpoints that need to be limited to one in-flight call.

Operations target one Resource by default. Catalogs declare `cardinality: many` only for Operations that read or write a collection.

Mutation Operations use HTTP status as their response by default and ignore the body as Resource state. Declare `response: resource` when the body is the authoritative Resource, or use a response mapping object when its collection, identity, or Resource envelope must be decoded. Read and list Operations consume Resource bodies by definition.

A display-name policy may define one `pointer` or an ordered `pointers` fallback list. The first pointer with a scalar value supplies the human-readable filename component; if none match, Taku falls back to the Resource ID. The `name_id` strategy appends up to the last eight characters of the Resource ID when that suffix is filename-safe, with an eight-character hash fallback for other IDs.

Filesystem Projection is separate from operation encoding. `split` converts one Resource Object into its canonical file tree; `merge` reconstructs it. A Many response must declare `response.collection` as `list` or `map`; `identity_pointer` captures an ID from a list entry, while an ID-keyed map supplies identity from its keys. `resource_pointer` then selects the Resource body inside each entry. This replaces structural guessing and keeps response collection wrappers out of the Canonical Representation.

For writes, `bundle.shape` independently declares a `list` or ID-keyed `map`, while `bundle.format` selects JSON or NDJSON encoding. An optional `bundle.multipart` wraps the encoded payload in a named form part. Map Bundles default to omitting identity from their values because the key carries it; List Bundles retain identity unless the Operation overrides `identity_in_body`. Kibana Skills use the built-in `frontmatter_markdown` projection, while Kibana Saved Objects use NDJSON Unbundling and an explicitly configured multipart NDJSON List Bundle.

```yaml
resource_types:
  skills:
    - version: ">=9.4.0, <10.0.0"
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: id, unique: true }
      filesystem:
        split: frontmatter_markdown
        merge: frontmatter_markdown
        frontmatter_markdown:
          document: SKILL.md
          body_pointer: /content
          referenced_files:
            pointer: /referenced_content
            path_pointer: /relativePath
            name_pointer: /name
            content_pointer: /content
            extension: md
      operations:
        list:
          method: GET
          path: /api/agent_builder/skills
          cardinality: many
          extract: /results
          response: { collection: list }
```

For `frontmatter_markdown`, every unclaimed YAML frontmatter value passes through to the Resource Object. The Markdown body and referenced files are the only extracted fields. Splitting serializes passthrough values back to frontmatter; comments and original YAML formatting are not part of the API round trip.

A Canonical Representation always describes one Resource and persists its ID at the reserved `/_taku/id` pointer even when the remote API puts that ID in a response-map key or request path. API identity fields remain ordinary wire fields and are never overwritten merely to persist Taku's identity. A path containing `{id}` omits `/_taku/id` and any matching API identity field from create, update, and upsert bodies by default; `identity_in_body` explicitly overrides that behavior. `body` may remain a static JSON request template, or a JSON Pointer string may select a narrower subtree after metadata and identity omission. For example, an ILM response keyed by policy name becomes one canonical object with `/_taku/id`, `version`, `modified_date`, and `policy` as siblings, while only `policy` is sent back:

```yaml
id: { pointer: /id, scope: universal }
metadata: { fields: [/version, /modified_date] }
operations:
  list:
    method: GET
    path: /_ilm/policy
    cardinality: many
    response: { collection: map }
  update:
    method: PUT
    path: /_ilm/policy/{id}
    body: /policy
```

For list entries such as `{name, component_template: {...}}`, `identity_pointer: /name` and `resource_pointer: /component_template` produce a flat canonical object containing the template fields plus `/_taku/id`; the response `name` is not copied into the Resource body. No create/update Transformation is required: the generic path-bound identity rule yields the API's direct single-template request body. Catalogs migrating from `kind: frame` should use Response Mapping for inbound collection envelopes and a `body` selector only when the write API genuinely accepts a narrower canonical subtree.

Update Mutation Mode is configured per Resource Type. Replace compares and owns the complete canonical document; Patch compares, pulls, and writes only the fields represented by the desired Resource. Target sensitive fields may tighten an Application's pre-persistence drops for a named Resource Type, but cannot remove identity or display-name state.

## Git Application Sources

Git sources use this layout:

```text
applications/
└── custom-application/
    ├── application.yaml
    ├── version-8.yaml
    └── version-9.yaml
```

Configure `application_source.location` in `.taku/project.yaml`, run `taku app refresh`, then install from the current offline cache. `taku update` is the only definition replacement workflow and refreshes each installed Git Application from its recorded source; `--from` explicitly switches eligible Applications to another source.
