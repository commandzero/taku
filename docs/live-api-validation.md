---
type: Guide
title: Live API validation
description: Explicit opt-in API validation, retained artifacts, and legacy test hazards.
generated: { by: openai-codex/gpt-6-sol, at: 2026-10-05T01:20:56Z }
---

# Live API validation

Use the opt-in [`live_api` validator](../tests/live_api.rs) for live functional validation of Taku/resource-control as a replacement for direct API workflows. Application definitions, Resource Type Catalogs, and declarative YAML cases drive the same generic harness; new application scenarios belong in fixtures, not application-specific Rust code. See the [live fixture guide](../tests/fixtures/live/README.md) for the exact schema, assertions, safety limits, and retained reports.

The `tests/fixtures/live/elastic.yaml` suite targets Elasticsearch and Kibana **9.4 with Agent Builder enabled**, covering pipeline create/update, a unique Kibana space, and skill/agent create/update. Supply `LIVE_ES_URL`, `LIVE_ES_AUTHORIZATION`, `LIVE_KB_URL`, and `LIVE_KB_AUTHORIZATION` externally in the process environment; authorization values are complete headers. Never put endpoints or secrets in fixture/credential files for this validator. There are no localhost or `.env` defaults.

From the repository root, **only after reviewing the fixture and catalogs, selecting disposable authorized services, and supplying those variables**:

```sh
mkdir -p /tmp/taku-live-artifacts
TAKU_LIVE_SUITE=tests/fixtures/live/elastic.yaml \
TAKU_LIVE_ARTIFACTS=/tmp/taku-live-artifacts \
TAKU_LIVE_ALLOW_MUTATIONS=1 \
cargo test --locked --test live_api live_api_suite -- --ignored --exact --nocapture
```

`TAKU_LIVE_ARTIFACTS` must name an existing directory. Each run retains its project and `report.json`; **there is no cleanup**, including after failure. HTTP requests and CLI invocations each have a 60-second timeout. Trust and review fixtures and catalogs: ID/path guardrails are not a security sandbox. Use disposable, authorized service instances and inspect the retained inventory before any separate manual cleanup.

`tests/fixtures/live/saved-objects.yaml` is a separate known-failure diagnostic for fresh saved-object absence detection, not a passing compatibility example. Acquisition, create, update, and no-op coverage is defined by fixture steps and strict status/body assertions, not automatically supplied by the harness; a passing suite is not complete API compatibility certification.

For offline transport contracts, use the [catalog fixture suite](../crates/resource-control/tests/fixtures/transport/suite.yaml):

```sh
cargo test -p resource-control --locked catalog_fixtures -- --nocapture
```

It checks literal inbound/outbound expectations against real catalogs with metadata tracking both disabled and enabled, without live services. Set `TAKU_TRANSPORT_SUITE` to use another suite.

## Legacy `live_elastic` tests — do not run

[`tests/live_elastic.rs`](../tests/live_elastic.rs) is the old, application-specific harness, not the recommended validator. It uses hardcoded localhost endpoints and reads `ELASTIC_API_KEY` from the repository `.env`. Its lifecycle tests perform automatic **DELETE cleanup**, including a fallback deletion attempt after assertion failure. Do not run this legacy harness or a blanket command that runs all ignored tests. Use the explicitly selected `live_api_suite` command above instead. The new suite format does not support legacy-format compatibility.
