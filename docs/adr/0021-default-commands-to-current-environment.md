---
type: Decision
title: "Default Commands to the Current Environment"
description: "Default Commands to the Current Environment."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Default Commands to the Current Environment

In a Multi-layout Project, an unqualified Resource command selects all managed Resources across all Targets in the current Environment. If no current Environment is available, it fails rather than choosing one. Operations spanning Environment boundaries require explicit repeated `--environment` selectors or `--all-environments`.

Commands share repeatable `--target`, `--type`, and `--id` selectors. The shorter `--type` is sufficient because Resource is already Taku's primary domain. Positional arguments remain command-specific, such as the Type accepted by `taku list`, and never ambiguously select either a Display Name or Resource ID. Exact selection uses the canonical Resource ID.
