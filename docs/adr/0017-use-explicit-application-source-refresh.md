---
type: Decision
title: "Use Explicit Application Source Refresh"
description: "Use Explicit Application Source Refresh."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Use Explicit Application Source Refresh

A Project may configure one Git Application Source that supplements the Applications embedded in the Taku binary. Application lookup precedence is an explicit `--from` source, then the Project-configured source, then embedded Applications. Taku must expose the selected and shadowed sources when names collide.

The configured source is contacted only by `taku app refresh` and `taku update`. `taku app` listing, `taku install`, and implicit installation by `taku app add` use the current local source cache and perform no implicit network access. A first installation therefore requires a previously refreshed cache when its Application is not embedded. Changing or refreshing a source never replaces an already vendored Application; updates remain explicit and record the resolved source commit and bundle checksum.

`taku update` updates all eligible installed Applications, while `taku update <application>` limits the operation. Updates preserve each installed Application's recorded source by default. Supplying `--from <source>` explicitly opts eligible Applications into a source switch. An Application is eligible when it is installed and present in the selected source; absent Applications are reported as skipped. Taku validates the complete update set and all affected Resources before atomically replacing any vendored definition, so one validation failure prevents every replacement in that invocation.
