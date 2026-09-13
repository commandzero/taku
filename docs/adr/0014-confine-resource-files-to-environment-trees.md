---
type: Decision
title: "Confine Resource files to validated Resource trees"
description: "Confine Resource files to validated Resource trees."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Confine Resource files to validated Resource trees

Taku will reject symlinked Resource files and directories, traversal, decoded filename escapes, and any Resource path resolving outside the Project's validated Single- or Multi-layout Resource tree, and it will transmit only files selected through a Resource Type. Explicitly configured credential and `.env` paths may live elsewhere because the user names them deliberately, but they remain provider inputs and can never be discovered or packaged as Resources.
