---
type: Policy
title: Documentation bundle
description: Bundle boundary and checks for Taku documentation.
generated: { by: openai-codex/gpt-6.1-sol, at: 2026-10-05T01:27:25Z }
---

# Documentation bundle

The entire `docs/` directory is the OKF 0.2 bundle. It includes ADRs, agent guides, repository policies, and generated indexes. Root README, CONTEXT, CHANGELOG, OpenSpec artifacts, prototypes, and scripts stay outside this bundle. Keep temporary audit reports and credentials outside it.

Every concept has type, title, description, and generated metadata. Preserve unknown metadata fields and existing source bodies. Change `generated` using the actual actor and time; record verification only when it occurs. No additional concept taxonomy or local schema applies.

Run `bash scripts/check-docs.sh` from the repository root. It checks complete-bundle conformance, lint, authored links, and index freshness using pinned OKF 0.2.7. Run `bash scripts/docs-index.sh` after adding, renaming, or changing concept titles or descriptions. This invokes OKF's index generator and converts its unordered entries to the team's adopted numbered-list convention; OKF 0.2.7 has no list-style option. Validation generates indexes in a temporary copy and never fixes authored files.
