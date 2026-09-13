---
type: Decision
title: "Treat unexplained remote absence as a Conflict"
description: "Treat unexplained remote absence as a Conflict."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Treat unexplained remote absence as a Conflict

Git is the reviewed record of accepted configuration, but Taku will not assume that a Resource present in Git should automatically resurrect one intentionally removed from a Target. Conclusive unexplained remote absence becomes a Presence Conflict by default; Push may resolve it as Restore and Pull as Delete, with precedence given to a command-line Missing Policy, then the Resource Type's workflow-specific default, then Conflict, while directionally inconsistent values are rejected.
