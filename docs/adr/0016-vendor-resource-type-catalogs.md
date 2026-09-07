---
type: Decision
title: "Vendor Resource Type Catalogs per Project"
description: "Vendor Resource Type Catalogs per Project."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Vendor Resource Type Catalogs per Project

Each Application's shared configuration and major-version Resource Type Catalogs will be copied by explicit `taku install` or `taku update` into the flat directory `.taku/applications/<application>/`. `application.yaml` declares shared transport behavior and ordered Version Endpoints; each `version-<major>.yaml` is self-identifying and contains complete version-qualified Resource Type Definitions. Minor additions and changes are represented by non-overlapping `version` constraints rather than overlays, keeping selection simple and making the exact behavior reviewable in Git.

Every file carries its schema version and top-level definition version. Each major catalog repeats `application.name` and declares its supported `application.version`; Resource Type definitions inherit that constraint when their own `version` is omitted, and omitted API Stability means Stable. Installation and update validate and checksum the complete bundle, replace it atomically, and never leave obsolete major files behind. Fetch, Pull, and Push never auto-update Applications.

`taku install <application>...` vendors definitions only and creates no Target. `taku app add <application> [name]` creates an Environment-specific Target whose name defaults to the Application name. When its Application is not installed, interactive confirmation or non-interactive `--yes` authorizes Taku to install the current available version and add the Target atomically; no implicit source refresh occurs.
