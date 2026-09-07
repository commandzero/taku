---
type: Decision
title: "Configure Push Git-State Policies Independently"
description: "Configure Push Git-State Policies Independently."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Configure Push Git-State Policies Independently

Push will inspect Git state without changing it and will evaluate selected uncommitted changes separately from selected untracked files. Project configuration exposes `push.uncommitted` and `push.untracked`, each with `block`, `confirm`, and `allow` policies.

Interactive execution defaults uncommitted changes to `confirm` and untracked files to `block`. Non-interactive execution defaults both conditions to `block`, so automation cannot publish unreviewed working-tree state without an explicit policy or command-line override. Precedence is command-line policy, then Project policy, then the execution-mode default. In non-interactive execution, `confirm` becomes `block`; `--yes` answers permitted confirmations but never weakens `block`.

These checks cover only Recognized Inputs selected by the Push. Taku neither acts on nor blocks for unrelated repository files. Push reports the current Git revision and relevant selected working-tree state in its plan.
