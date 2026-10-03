# Live API validator fixtures

The generic [`tests/live_api.rs`](../../live_api.rs) harness validates Taku/resource-control as a functional replacement for direct API workflows. Application behavior belongs in Application definitions, Resource Type Catalogs, and declarative suites—not application-specific Rust branches. This is the current suite format; no legacy-format compatibility is supported.

## Suites and prerequisites

- `elastic.yaml`: Elasticsearch and Kibana **9.4 with Agent Builder enabled**; pipeline create/update, a unique Kibana space, and skill/agent create/update.
- `saved-objects.yaml`: separate **known-failure diagnostic** for fresh saved-object absence detection. An unexpected absence response must fail honestly, not be accepted as success or skipped. There is no expected-failure flag in the schema.

These fixtures do not automatically certify complete compatibility. Acquisition, create, update, and no-op coverage depends on the steps and assertions actually authored. Use independent HTTP readbacks and explicit CLI/file expectations to establish those behaviors, rather than treating a successful process exit as proof.

Run only against authorized, disposable service instances. Have the Rust toolchain and Git available, and supply these variables externally in the process environment:

| Variable | Meaning |
| --- | --- |
| `TAKU_LIVE_SUITE` | Path to an existing YAML suite; no default. |
| `TAKU_LIVE_ARTIFACTS` | Existing directory in which a unique run directory will be created; no default. |
| `TAKU_LIVE_ALLOW_MUTATIONS` | Must be exactly `1`, even for a suite that only probes. |
| `LIVE_ES_URL` | Elasticsearch base URL used by the Elastic fixture. |
| `LIVE_ES_AUTHORIZATION` | Complete Elasticsearch Authorization header value, including its scheme. |
| `LIVE_KB_URL` | Kibana base URL used by the Elastic fixtures. |
| `LIVE_KB_AUTHORIZATION` | Complete Kibana Authorization header value, including its scheme. |

The `LIVE_*` names are chosen by the suites, not hardcoded into the harness. Every referenced variable must be nonempty. There are no localhost defaults, `.env` loading, or credential files. Never store secrets in fixtures, catalogs, generated projects, or other files for the validator. Inject them through your external environment/secret provider. URLs must be HTTP(S), with a host and without embedded credentials, query, or fragment; a base-path prefix is allowed.

From the repository root, after reviewing the suite and catalogs and supplying the endpoint/auth variables:

```sh
mkdir -p /tmp/taku-live-artifacts
TAKU_LIVE_SUITE=tests/fixtures/live/elastic.yaml \
TAKU_LIVE_ARTIFACTS=/tmp/taku-live-artifacts \
TAKU_LIVE_ALLOW_MUTATIONS=1 \
cargo test --locked --test live_api live_api_suite -- --ignored --exact --nocapture
```

For the separate saved-object diagnostic, use the same command with `TAKU_LIVE_SUITE=tests/fixtures/live/saved-objects.yaml`; expect a failing result until the absence-detection issue is resolved. Inspect the report rather than weakening assertions. Do not run the legacy `live_elastic` harness: it performs automatic DELETE cleanup. Avoid blanket invocations of all ignored tests.

## Exact suite schema

Unknown fields are rejected at every suite, target, case, step, and assertion level. The root has only `targets` and `cases`; there is no suite `schema_version` field. Both collections must be nonempty.

| Object | Fields |
| --- | --- |
| Suite | `targets`: map of target names to target definitions; `cases`: ordered list of cases. |
| Target | Required `application`: suite-relative path ending in `application.yaml`; required `url_env`: environment variable name; optional `authorization_env`: environment variable name; optional `headers`: string-to-string map, defaults to `{}`. |
| Case | Required `name`, `target`, `resource_type`, and `steps`; optional `namespace`. |

A target's `application.yaml` must have sibling `version-*.yaml` catalogs. Catalog Application names must match; definitions for the same Application name must agree across targets. The harness copies these definitions into the generated project and uses Taku's real catalog parser. A case must reference a configured target and a Resource Type present in its catalogs; live version discovery still determines the applicable catalog at runtime.

Target names, case names, and Resource Type names start with an ASCII alphanumeric character and contain only ASCII alphanumerics, `.`, `_`, or `-`. Case names must be unique. A rendered namespace follows the same component rules. Environment variable names start with an ASCII letter or `_`, followed by ASCII letters, digits, or `_`.

Target headers are shared by direct HTTP steps and the CLI target configuration. Use `authorization_env` for credentials: `authorization`, `proxy-authorization`, `cookie`, and `host` header overrides are forbidden, case-insensitively. Headers are for non-secret values such as Kibana's `kbn-xsrf` header.

### Templates

Templates expand in parsed case string values and object keys, including paths, JSON request bodies, file contents, assertions, and namespace:

- `{{run}}`: `taku-live-<Unix timestamp in nanoseconds>-<process ID>`, shared across cases.
- `{{id}}`: `<run>-<case name>`, the exact Resource ID selected by every CLI step in that case.

Expansion is literal substitution, not shell expansion or a general template language. Duplicate object keys introduced by expansion are rejected. Target definitions cannot contain either template. Use `{{run}}` for a shared unique Kibana space and `{{id}}` for a case's resource. CLI IDs are assigned by the harness, not supplied in a step.

### Steps

Each step is an object tagged by `kind`. Only the fields listed here are accepted:

| `kind` | Fields and behavior |
| --- | --- |
| `http` | Required `method`, `path`, `status`; optional `body` (JSON value) and `assertions` (defaults to `[]`). Methods: `GET`, `POST`, `PUT`, `PATCH` only. Sends a JSON body when present; requires the exact numeric status, in `100..599`. Assertions inspect the response body. |
| `cli` | Required `command`; optional `assertions` (defaults to `[]`). Commands: `add`, `fetch`, `pull`, `push`, `status` only. Requires successful exit; assertions inspect stdout, not stderr. |
| `file` | Required `path` and `assertions`. Reads a UTF-8 file in the generated project and asserts on its contents. |
| `write` | Required `path` and string `content`. Creates parent directories and writes/overwrites the project file. |
| `replace` | Required `path`, nonempty string `old`, and string `new`. Requires exactly one occurrence of `old`, then replaces it. |

HTTP paths must start with `/`, not `//`, and cannot contain backslashes, fragments, or control characters. They are appended to the target base URL and must preserve its origin. Direct HTTP redirects are disabled. There is no step-level target, header, arbitrary CLI-argument, deletion, or cleanup override.

Every case must begin with a `GET` or `POST` HTTP step expecting **404**, with literal `{{id}}` in its path or body before expansion. A non-GET HTTP step must include `{{id}}` or `{{run}}` in its path or body. Every case must contain at least one scoped CLI step. Each `push` must be immediately followed by a `GET` or `POST` HTTP step expecting a specific 2xx status, containing at least one assertion, and selecting `{{id}}` in its path or body. This supplies independent API readback; fixture authors must ensure it actually checks the intended resource and fields.

CLI steps run the compiled binary as:

```text
taku --non-interactive --output json <command> <target> <resource_type> <id> --environment live
```

If set, the case namespace adds `--namespace <namespace>`. `pull` adds `--yes --missing conflict` so catalog defaults cannot delete missing local resources. `push` adds `--yes --missing restore --uncommitted allow --untracked allow`. There is no configurable expected failure/exit code. Each HTTP request and CLI invocation has a **60-second timeout**, not a single 60-second budget for the entire suite.

File paths are relative to the generated project, without absolute paths, `.`/`..`, `.git`, or `.taku` components. Symlink traversal is rejected. File steps cannot edit project configuration or catalogs.

### Assertions

Assertions are objects tagged by `kind`:

| `kind` | Required fields | Meaning |
| --- | --- | --- |
| `json` | `pointer`: string; `equals`: JSON value | Parse the entire output as JSON and require exact equality at the JSON Pointer. A missing pointer is a failure, including when expecting `null`. |
| `contains` | `text`: string | Require the literal substring. |
| `not_contains` | `text`: string | Reject the literal substring. |

JSON Pointers are empty for the whole document or start with `/`; only `~0` and `~1` escapes are valid. Assertions are conjunctive and exact: there are no regexes, wildcard statuses, coercions, or automatic error acceptance. HTTP status is checked before body assertions; nonzero CLI exit fails before assertions. Empty assertion lists provide no body/content coverage. Prefer explicit JSON field checks for remote data and explicit CLI/file checks for acquisition and no-op evidence. For an updated, already-acquired resource, check `status: in_sync` after fetch and before pull; otherwise pull can hide round-trip loss. For fresh creation where the server adds defaults, assert all authored fields on independent readback and again in the pulled files before checking a no-op.

## Safety, execution, and retained evidence

**Trust and review both fixtures and catalogs. This is not a security sandbox.** A marker in a path/body does not prove that an API mutation is restricted to that resource. POST may mutate even when used as a probe/readback, and catalogs determine CLI API semantics. The harness has no DELETE step or automatic cleanup, but it cannot prove arbitrary API behavior harmless. Never use production targets or unreviewed configuration.

Schema, environment values, catalogs, rendered paths, and assertions are checked before execution. The harness creates a unique run directory, initializes a single-layout Git/Taku project with Environment `live`, installs the selected catalogs, and runs cases and steps in order. A failed step skips the rest of that case; subsequent cases still run. Any failed case fails the suite.

After execution starts, the artifact layout is:

```text
<TAKU_LIVE_ARTIFACTS>/<run>/
  report.json
  project/
    .git/
    .taku/
    ... resource files and other state produced by the steps
```

The report is written before setup and updated through execution. It contains:

- Run identifier and overall status; initialization error if applicable.
- Setup command observations (`git init`, `taku init`, `taku app`).
- Case inventory: name, target, Application, Resource Type, namespace, generated ID, and status.
- Application definition/catalog versions and target environment-variable names.
- Per-step rendered specification (including path/command, request body, and assertions), kind, status, error when present, and captured HTTP status, exit code, stdout, and stderr.

Setup/steps not reached can remain pending or be skipped. The inventory lists all cases, not proof that every listed resource was created, nor a complete inventory of incidental API side effects. Early settings/schema failures occur before a run directory/report exists. The report retains rendered step inputs; the project retains the exact application definitions and catalogs used. Keep the original suite alongside your interpretation of results as well.

**No remote or local cleanup runs on success or failure.** Inspect the retained project and report, then arrange any manual cleanup separately using the actual IDs/namespaces and service inventory. Failed runs may leave partially created resources and spaces.

Persistent target configuration stores environment variable names and a dummy URL, not resolved endpoints/credentials. Reports omit request headers and environment maps. Known environment values, authorization tokens, and their JSON-escaped forms are redacted; after each CLI invocation, project files are scrubbed and the step fails if a known value was persisted. This is best-effort matching, not arbitrary secret discovery. Protect retained artifacts and inspect them before sharing; remote response data can itself be sensitive.

## Offline transport fixtures

The separate [transport suite](../../../crates/resource-control/tests/fixtures/transport/suite.yaml) checks catalog transformations without remote services:

```sh
cargo test -p resource-control --locked catalog_fixtures -- --nocapture
```

Set `TAKU_TRANSPORT_SUITE` to an alternative suite path. Its schema is distinct: `fixtures` contains suite-relative `catalog` paths and `cases`; each case declares `name`, `resource_type`, zero-based `variant`, `input`, `expected_inbound` (`untracked` and `tracked`), and nonempty `expected_outbound` entries keyed by `create`, `update`, or `upsert`. Expected documents are independent literals. Each declared write is checked with metadata tracking disabled and enabled. Passing these offline contracts is not a claim that a live API accepts those payloads.
