---
type: Decision
title: "Drop Sensitive Fields Before Persistence"
description: "Drop Sensitive Fields Before Persistence."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Drop Sensitive Fields Before Persistence

Resource Types may declare exact JSON-pointer-like Sensitive Fields, and Project configuration may add stricter pointers without removing catalog declarations. Taku drops these fields from parsed remote responses before writing a Canonical Resource, Observed State cache, output document, journal, or any other persistent representation. A Sensitive Field is therefore absent rather than stored with a mask.

Validation rejects Sensitive Fields that overlap Resource identity or other required canonical state and rejects a Canonical Resource that contains a declared Sensitive Field. If a response cannot be safely parsed and sanitized, diagnostics omit its raw body. This protects configured response fields only; it does not discover arbitrary secrets or inject secret values into outbound Resources.
