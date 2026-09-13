---
type: Decision
title: "Separate observation, adoption, and Git history"
description: "Separate observation, adoption, and Git history."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Separate observation, adoption, and Git history

Taku will separate `fetch`, which observes remote state without changing desired Resources, from `status` and `diff`, which compare state, and `pull`, which explicitly adopts Observed State into the working tree after a reviewable plan and confirmation. Taku may use Git-like operational vocabulary, but it will not change repository history; Git remains responsible for commits, branches, and remote repository synchronization so adopting live state cannot silently redefine the source of truth.
