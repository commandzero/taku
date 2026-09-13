---
type: Decision
title: "Separate resource-control from Taku"
description: "Separate resource-control from Taku."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Separate resource-control from Taku

The reusable, protocol-neutral engine will be named `resource-control`, while `taku` will be the user-facing reference CLI and product. This names the library after the durable resource-control abstraction rather than today's REST transport or synchronization use case, allows opinionated consumers such as `kibob` to reuse it, and keeps Taku's branding independent of implementation details.
