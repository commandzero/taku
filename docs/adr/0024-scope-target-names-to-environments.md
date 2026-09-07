---
type: Decision
title: "Scope Target Names to Environments"
description: "Scope Target Names to Environments."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Scope Target Names to Environments

Target names are Environment-specific rather than Project-wide because corresponding remote instances commonly have different operational names, such as `dev/es-dev` and `prod/es-prod`. `taku app rename <old> <new>` changes only the current or explicitly selected Environment's Target configuration and Resource tree, preserves Resource IDs, invalidates Target-bound caches and incomplete Push Journals, performs no remote calls, and leaves Git changes unstaged.
