---
type: Decision
title: "Model optional Resource namespaces explicitly"
description: "Model optional Resource namespaces explicitly."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Model optional Resource namespaces explicitly

Applications may isolate some Resource Types inside independently managed Namespaces while leaving other Resource Types global to the Target. A Resource Type therefore explicitly opts into namespacing; namespaced Resources use `<environment>/<target>/<namespace>/<resource-type>/<name>` in Multi layouts and `<target>/<namespace>/<resource-type>/<name>` in Single layouts, while non-namespaced Resources retain the existing path. The Namespace is part of Resource identity, and `default` is always an explicit directory when namespacing is enabled. Namespace lifecycle remains an ordinary Resource concern handled by an application-defined Resource Type, such as Kibana `spaces`, rather than a special Taku command.

This avoids hiding scope in Target configuration, permits one Target to manage multiple Namespaces, and makes namespace-specific changes reviewable in Git. An Operation's `path` always describes the default Namespace route. Applications translate named Namespaces by optionally prepending `namespace.prefix` and appending `namespace.suffix`; the explicit `default` Namespace uses `path` unchanged. This keeps the common route canonical without duplicating complete default and named paths.
