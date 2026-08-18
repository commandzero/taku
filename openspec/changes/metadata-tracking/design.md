## Context

See `proposal.md` for motivation and `specs/resource-metadata/spec.md` for the behavioral contract.

Resource Type Definitions currently use an ordered `transformations` list. Inbound processing applies it forward: `remove` deletes a response field and `omit` does nothing. Outbound processing applies it in reverse: `omit` deletes a request field and `remove` does nothing. Embedded catalogs therefore express API-owned fields as inbound `remove` transformations, which makes them indistinguishable from structural response normalization and gives a repository directory no way to retain them in Git.

Taku's desired state deliberately uses the filesystem as its single representation rather than mirroring Environment, Target, Namespace, and Resource Type hierarchy in centralized configuration. In a Single layout, a Target root is `<target>`; in a Multi layout it is `<environment>/<target>`. A non-namespaced Resource Type directory is directly beneath that root. A namespaced Type directory is `<target-root>/<namespace>/<resource-type>`. Flat Resources are files within that directory, while projected Resources are child directories.

Current flat inventory treats every non-marker file in a Resource Type directory as a Resource. Projected Resource merge recursively treats every non-document file inside an individual projected Resource directory as API content, and projected Pull atomically replaces that directory. Push Git-state checks currently include only selected Resource and Deletion Marker paths. Sidecar hints therefore require explicit recognition at the filesystem, binding, and Git-state seams rather than merely adding YAML parsers.

## Goals / Non-Goals

**Goals:**

- Keep Application catalogs authoritative for which Canonical fields are API-owned metadata.
- Put repository tracking choice beside the Target or Resource Type directory it modifies, with no repeated hierarchy selectors.
- Make whole Target and Resource Type directories self-describing when copied or reviewed in Git.
- Resolve one deterministic physical-directory policy for flat, projected, namespaced, Single, and Multi layouts.
- Keep API-owned metadata out of every outbound payload while making opted-in metadata reviewable in Git.
- Treat hints as selected configuration inputs with the same safety posture as Resources.

**Non-Goals:**

- Adding metadata policy to `.taku/project.yml` or an Application definition.
- Configuring one logical Resource Type across all Targets or Namespaces from a central selector tree.
- Configuring individual Resources; `.resource.yaml` configures the Resource Type directory containing them.
- Allowing arbitrary local overlays of catalog identity, Operations, transformations, transport, projections, lifecycle, concurrency, dependencies, or Sensitive Fields.
- Automatically generating hint files or copying them through Promotion.
- Sending metadata back when an API happens to accept it or making metadata locally authoritative.

## Decisions

### 1. Add two closed hint manifests whose identity comes entirely from placement

Both manifests use the same minimal shape initially:

```yaml
schema_version: 1
metadata:
  track: true
```

`.target.yaml` is valid only at a known Target root:

```text
# Single
<target>/.target.yaml

# Multi
<environment>/<target>/.target.yaml
```

`.resource.yaml` is valid only in a physical Resource Type directory:

```text
# Non-namespaced Single / Multi
<target>/<type>/.resource.yaml
<environment>/<target>/<type>/.resource.yaml

# Namespaced Single / Multi
<target>/<namespace>/<type>/.resource.yaml
<environment>/<target>/<namespace>/<type>/.resource.yaml
```

The files do not repeat names already encoded by these paths. Use dedicated `TargetHints` and `ResourceHints` schemas with `deny_unknown_fields` even if their initial nested metadata shape is shared. This keeps the names self-documenting, permits scope-specific hints later, and prevents them from becoming untyped YAML merge patches.

The exact filenames are reserved beneath managed Target trees. A reserved name at an invalid level—including inside an individual projected Resource directory—is an error rather than ordinary Resource content.

**Alternative considered:** Store an Application/Resource Type policy tree in `.taku/project.yml`. Rejected because it duplicates the desired-state hierarchy and separates configuration from the directory it modifies.

**Alternative considered:** Use one `.taku.yaml` at every level. Rejected because explicit filenames make valid placement and scope reviewable and reduce ambiguity inside projected Resource directories.

**Alternative considered:** Put `.resource.yaml` inside each individual Resource directory. Rejected because flat Resources are files, projected Resources are directories, projected merge would ingest the hint as API content, and Pull replaces projected directories atomically.

### 2. Scope hints to physical directories and prefer the closest value

Resolve effective tracking for an inventory or remote item from its destination path:

1. `metadata.track` in the physical Resource Type directory's `.resource.yaml`;
2. `metadata.track` in the enclosing Target root's `.target.yaml`;
3. `false`.

This intentionally changes the earlier logical scope into physical scope. `.target.yaml` belongs to one Target instance, not every Target using the same Application. For namespaced Types, `.resource.yaml` belongs to one Namespace/Type directory, not the same Type across all Namespaces. A Target hint supplies the convenient broad default; Resource Type directories supply local exceptions.

Create a single path-aware hint resolver that accepts validated Project layout, Environment, Target, optional Namespace, and Resource Type. Inventory, remote Add, Fetch, reconciliation, Push, Deletion Marker handling, and Promotion call that resolver rather than independently walking parent directories. It reads only the two exact expected paths; it does not perform open-ended ancestor discovery.

**Alternative considered:** Search upward for arbitrary hint files. Rejected because Namespace and Target boundaries would become implicit, typo handling would be unclear, and unrelated ancestors could change behavior.

**Alternative considered:** Make one namespaced Resource Type hint apply across all Namespaces. Rejected because no single physical Resource Type directory owns that scope; implementing it would reintroduce a keyed overlay elsewhere.

### 3. Treat hint manifests as first-class recognized configuration inputs

Inventory must distinguish three kinds of children in a physical Resource Type directory: `.resource.yaml`, Deletion Markers, and Canonical Resource files/directories. Flat inventory skips the reserved hint after parsing it as configuration. Projected inventory considers only child directories as Resources and keeps the parent hint outside projection merge/split.

Validation also discovers reserved filenames that are not reached through normal inventory so they cannot be silently ignored. A `.target.yaml` must bind to a Target declared for that layout and Environment. A `.resource.yaml` must bind to a Resource Type available in the selected definition; its Namespace position must match the Type's namespacing. Unavailable-Type checks treat a validly placed `.resource.yaml` as a recognized input, just as they treat Resources and Deletion Markers, while never parsing it as a Resource.

Reject symlinked hints and symlinked path components using the existing managed-input policy. Parse schema versions and unknown fields before network or filesystem mutation.

**Alternative considered:** Ignore dotfiles generically. Rejected because unrelated hidden files would become silently special and malformed or misplaced Taku hints could escape validation.

### 4. Keep hint lifecycle separate from Resource lifecycle

Add, Fetch, and Pull read hints but never materialize or rewrite them. Remove and Forget operate on individual Resource paths or Deletion Markers and leave the parent Resource Type directory intact when it contains `.resource.yaml`. Projected split replaces only the individual projected Resource child directory, so the parent hint survives naturally.

Target Rename already moves the complete Target root and therefore carries `.target.yaml` and every descendant `.resource.yaml` without rewriting identity fields—there are none. Directory configuration remains valid at the new path after normal Target binding and observation invalidation.

Treat a Resource Type directory containing only `.resource.yaml` as a valid configuration-bearing directory with an empty inventory. This allows a repository to configure a Type before Add and preserves intent after Forget removes its last Resource.

**Alternative considered:** Delete empty Resource Type directories automatically. Rejected because the hint makes the directory semantically non-empty and intentionally self-contained.

### 5. Compile catalog metadata plus directory policy into canonical processing

Metadata pointers address the value after configured inbound Resource Type transformations. API-owned metadata is never writable, regardless of tracking choice. Conceptually append synthetic transformations:

- always append `omit` for every metadata pointer so outbound encoding removes it;
- when effective `track` is `false`, also append `remove` so inbound canonicalization drops it;
- when effective `track` is `true`, do not add the inbound removal.

Appending preserves the required order. Inbound structural transformations run first, then untracked metadata is removed. Outbound processing reverses the synthetic list first, removing metadata before Resource Type transformations are reversed and before Operation framing or bundling.

Implement this through metadata-aware canonical helpers parameterized by the resolved hints rather than mutating serialized catalog transformations. Missing pointers are no-ops. General Status/Diff/Pull compares the resulting Canonical Representation: tracked metadata remains observable and pullable. Push equality removes declared metadata from both desired and observed values for Replace and Patch modes, preventing metadata-only no-op writes even if an untracked field was manually added to a Canonical file.

**Alternative considered:** Omit metadata outbound only when tracking is enabled. Rejected because the catalog declaration says the field cannot safely be posted back; outbound safety must not depend on the Canonical file being clean.

**Alternative considered:** Exclude tracked metadata from every comparison. Rejected because metadata-only remote changes would never become visible or pullable.

### 6. Bind raw applicable hints into observation and Push safety

Hash each expected hint path, its presence or absence, and its bytes into the Observed State binding for the affected Target/Resource Type directory. Including absence ensures creating a hint after Fetch invalidates the observation even if its explicit value equals the default. Include the same material in Push plan and journal bindings while retaining original Canonical input hashes.

Extend selected Git-state paths with the applicable Target and Resource Type hint paths. A scoped Push includes only the Target hints and physical Resource Type hints that can affect selected Resources or Deletion Markers; unrelated modified hints do not block. Broad Push collects applicable hints across its selected inventory and markers. A hint-only directory with no selected Resource causes no remote work, but `validate` still validates it.

Changing a hint requires Fetch and Pull before Push: false-to-true needs a new observation that retains metadata, while true-to-false needs Pull to remove it from Git. This uses the existing structural-invalidity gate rather than inventing a special migration state.

**Alternative considered:** Bind only the resolved boolean. Rejected because adding, deleting, or reformatting a recognized configuration input should remain reviewable and journal-safe even when the effective value is unchanged.

### 7. Make Promotion destination-owned

`taku promote` continues copying Resource snapshots, not directory configuration. Resolve hints at the destination physical Resource Type directory and normalize the promoted Canonical value before writing it. If the source tracked metadata and the destination does not, drop it. If the destination tracks metadata but the source does not contain it, do not synthesize it; a destination Fetch/Pull may add remote metadata later.

Manual copying of an entire Target or Resource Type directory naturally carries its hints, preserving the filesystem's self-contained character. Command-driven Promotion intentionally keeps destination policy authoritative, matching the existing destination-owned Promotion model.

**Alternative considered:** Copy source hints during Promotion. Rejected because it would overwrite destination policy and turn a Resource promotion into configuration mutation.

### 8. Validate catalog metadata against structural and sensitive state

Catalog validation requires non-empty, valid, non-root pointers with no duplicates or ancestor/descendant overlap. Reject overlap with ID, display-name, catalog Sensitive Fields, structural extraction/reconstruction state, filesystem projection state, or explicit Resource Type `remove`/`omit` transformations.

Target-added Sensitive Fields are known only with Project and hint context. Reject an effective `track: true` when declared metadata overlaps those additions. With tracking disabled, Sensitive Field removal remains the stronger persistence rule, but catalog-declared Sensitive Fields still cannot also be declared metadata because that catalog would advertise unusable tracking semantics.

### 9. Preserve existing schemas and migrate only API-owned removals

Do not change `.taku/project.yml`. The new sidecars each start at `schema_version: 1`; Resource Type `metadata` is optional within the existing catalog schema. Increment affected embedded catalog definition versions so installed Application updates remain explicit and stale observations invalidate.

Audit existing `remove` transformations rather than converting mechanically. Move response-only provenance and server-maintained fields into `metadata.fields`. Leave structural normalization, response-envelope extraction, inserted identity, and outbound identity omission as transformations. With no sidecars, effective tracking is false and existing Canonical output remains byte-equivalent.

## Risks / Trade-offs

- **[Physical scope differs from logical Application/Type scope]** → Document that Target instances and Namespace/Type directories are independent; use `.target.yaml` for a broad local default.
- **[Application-wide opt-in requires one hint per Target]** → Accept this as the cost of directory locality and avoid reintroducing a central selector tree.
- **[Reserved filenames could collide with projected API content]** → Reserve only the two exact names and reject them at invalid levels before projection merge.
- **[Status and Push can report different states for tracked metadata]** → Document observable-versus-writable state and test metadata-only drift.
- **[Server metadata may churn and create noisy Git diffs]** → Default to false and require an explicit directory-local opt-in.
- **[Hint edits might evade existing selected Git checks]** → Add applicable sidecars as explicit selected paths and bind raw presence/content into observations and journals.
- **[Promotion could carry stale source metadata]** → Normalize with destination hints and never copy source sidecars.

## Migration Plan

1. Add Resource Type metadata declarations and the two closed sidecar schemas without changing Project metadata.
2. Add deterministic hint path resolution, strict discovery/validation, reserved-name handling, and projected-resource exclusions.
3. Add raw hint binding and selected Git-state coverage before enabling hints to affect canonicalization.
4. Add inbound removal, unconditional outbound omission, direction-aware comparison, and destination Promotion normalization.
5. Audit and migrate only API-owned `remove` transformations in embedded Elasticsearch and Kibana catalogs; increment affected definition versions.
6. Update documentation and verify repositories without sidecars produce byte-equivalent Canonical Resources and request payloads.
7. Run formatting, Clippy, the full non-ignored suite, focused live tests, and strict OpenSpec validation.

Rollback removes the sidecar-aware code and migrated catalog declarations. Repositories must remove `.target.yaml` and `.resource.yaml` before using an older binary because older flat inventory may parse `.resource.yaml` as a Resource. Repositories that tracked metadata must also remove those fields from Canonical Resources before an older catalog can consider them safe and in sync.
