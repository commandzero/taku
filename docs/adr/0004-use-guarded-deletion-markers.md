---
type: Decision
title: "Use guarded deletion markers for partial inventories"
description: "Use guarded deletion markers for partial inventories."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Use guarded deletion markers for partial inventories

Taku manages partial remote inventories, so omission means unmanaged and never implies remote deletion. Deletion instead requires an Environment-specific marker carrying an available version, revision, or canonical fingerprint; Taku deletes only while that guard matches, consumes the marker after confirmed absence, retains it after uncertain results, turns stale or mismatched markers into Conflicts, and allows `forget` to remove one without touching the remote Resource.
