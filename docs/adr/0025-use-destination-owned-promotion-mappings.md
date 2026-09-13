---
type: Decision
title: "Use Destination-Owned Promotion Mappings"
description: "Use Destination-Owned Promotion Mappings."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Use Destination-Owned Promotion Mappings

Each higher Environment may declare one `from` Environment as its default upstream. Within that destination Environment, a Target may declare `from` to identify a differently named Target in the upstream Environment; omission attempts the same name. The Environment precedence graph must be acyclic, and a resolved Target pair must use exactly compatible installed Applications and Target Profiles.

The current Environment is the default Promotion destination. Plain `taku promote` follows its configured mappings, while `--from` and `--to` override Environment selection and `--from-target` and `--to-target` override a Target pair. A selected Target with no destination or no resolved upstream mapping is warned and skipped rather than guessed from its Application name. Promotion continues for independently resolved Target pairs.
