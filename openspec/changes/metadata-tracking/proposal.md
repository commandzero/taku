## Why

Taku currently models API-owned fields such as creators and modification timestamps as ordinary inbound transformations, so they are removed before Git even when they would provide useful provenance. Application catalogs need to identify metadata, while each repository directory must be able to choose whether to track it without duplicating the Project hierarchy in centralized configuration or sending metadata back to the remote API.

## What Changes

- Add an optional metadata declaration to each Resource Type Definition, containing exact JSON pointers for API-owned fields.
- Add optional, tracked `.target.yaml` and `.resource.yaml` typed hint manifests inside the Target and Resource Type directories they modify; the filesystem path supplies their Environment, Target, Namespace, Application, and Resource Type identity without repeating that hierarchy in file content.
- Let `.target.yaml` select metadata tracking for every Resource Type under one Target and let the closer `.resource.yaml` override it for the Resources in one physical Resource Type directory. Namespaced Resource Type overrides are therefore Namespace-local.
- Default absent metadata hints to `track: false`, which removes metadata before comparison and Pull and preserves the current Git behavior. `track: true` retains metadata in observed and Canonical Resources but removes it from outbound create, update, and upsert payloads.
- Reserve the exact sidecar filenames as recognized configuration inputs, validate their placement and closed schema, and include applicable sidecars in Git-state checks, observation bindings, and Push journal safety.
- Keep sidecars independent from Resource lifecycle: Add and Pull honor but never create or rewrite them, Remove and Forget preserve them, and Promotion uses destination-side hints rather than copying source hints.
- Reuse the existing bidirectional transformation pipeline as the execution mechanism: Pull-time metadata behaves like inbound `remove`, while Push-time metadata behaves like outbound `omit`.
- Validate catalog metadata pointers, directory-local hints, and interactions with identity, display-name, projections, sensitive fields, and explicit transformations before any network or filesystem mutation.
- Update the embedded Elastic Application catalogs to express API-owned metadata through the new declaration without changing their default persisted output.

## Capabilities

### New Capabilities

- `resource-metadata`: Defines catalog metadata declarations, directory-local Target/Resource Type hint manifests, precedence and lifecycle, Pull-time removal, Push-time omission, safety binding, and compatibility with existing transformations.

### Modified Capabilities

None. This repository does not yet contain main OpenSpec capabilities.

## Impact

- Affects the recognized filesystem grammar, Resource Type Catalog schema, configuration validation and safety binding, inventory and Git-state selection, inbound observation/canonicalization, comparison and Pull, outbound Push encoding, Promotion, embedded Elasticsearch and Kibana catalogs, and related CLI/live tests.
- Introduces two closed, versioned hint-file schemas without adding Project hierarchy overlays or permitting local overrides of catalog Operations, transformations, transport, or identity behavior.
- Changes no `.taku/project.yml`, Target, Canonical Resource, Baseline, Observed State, Journal, or CLI command syntax. The same installed Application may therefore track metadata differently in separate Target or Resource Type directories.
