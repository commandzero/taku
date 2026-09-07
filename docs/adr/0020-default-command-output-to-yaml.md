---
type: Decision
title: "Default Command Output to Structured YAML"
description: "Default Command Output to Structured YAML."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Default Command Output to Structured YAML

Taku commands will write a versioned structured YAML envelope to standard output by default and will support explicit JSON through `--output json`. This makes ordinary output both human-readable and economical for agent parsing while preserving a machine-contract alternative. Prompts and diagnostics go to standard error, and secret-bearing values must be redacted in every format and error path.

Inspection commands mirror Git exit behavior: differences alone do not make `status` or `diff` fail. An explicit `--check` or `--exit-code` mode returns a distinct nonzero status for differences, while conflicts, invalid configuration, and operational failures remain distinguishable from ordinary drift.
