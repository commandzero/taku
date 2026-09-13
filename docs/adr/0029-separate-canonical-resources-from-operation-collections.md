---
type: Decision
title: "Separate canonical Resources from operation collections"
description: "Separate canonical Resources from operation collections."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Separate canonical Resources from operation collections

Taku stores exactly one flat Resource Object per Canonical Representation, including its Resource ID at the reserved `/_taku/id` pointer, rather than preserving response collection envelopes or request-only wrappers. API identity fields remain wire fields and are never overwritten just to persist Taku's identity. An Operation Response declares whether success is conveyed by HTTP Status, a direct Resource body, or a Response Mapping; mutations default to Status so acknowledgement bodies cannot be mistaken for Resource state. Response Mapping explicitly decodes remote Lists and ID-keyed Maps into Resource Objects, while Bundling independently rebuilds List or Map request collections; path-bound identity and an optional Operation body selector produce the final single-Resource request body. This keeps repository state aligned with the meaningful resource body while retaining enough identity and metadata for review.
