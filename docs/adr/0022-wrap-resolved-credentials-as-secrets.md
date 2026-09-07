---
type: Decision
title: "Wrap Resolved Credentials as Secrets"
description: "Wrap Resolved Credentials as Secrets."
generated: { by: codex/gpt-6, at: 2026-09-07T05:34:57Z }
---

# Wrap Resolved Credentials as Secrets

Taku will wrap resolved credential strings in `redact::Secret<String>` immediately at Environment and dotenv provider boundaries and expose them only while constructing transport authentication. Credential fields are omitted from output data models rather than serialized as implementation-specific redaction markers. Sentinel-value tests must verify absence from default YAML, explicit JSON, errors, journals, Observed State caches, and debug output.

This wrapper reduces accidental formatting and serialization; it is not encryption, automatic log scrubbing, or discovery of sensitive values inside arbitrary Resource payloads. Parser and transport errors still require sanitized diagnostics, and Taku will not use plaintext opt-in serialization for credentials.
