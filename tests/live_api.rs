//! Opt-in, fixture-driven integration checks. Fixtures and application catalogs must be
//! reviewed: ID/path guardrails are not a security sandbox for arbitrary API semantics.
//! No live defaults, cleanup, deletion commands, or application-specific behavior.

use anyhow::{Context, Result, bail, ensure};
use assert_cmd::Command;
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[path = "support/live_api_checks.rs"]
mod checks;

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    targets: BTreeMap<String, Target>,
    cases: Vec<Case>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Target {
    application: PathBuf,
    url_env: String,
    authorization_env: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    target: String,
    resource_type: String,
    namespace: Option<String>,
    steps: Vec<Step>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Http {
        method: HttpMethod,
        path: String,
        body: Option<Value>,
        status: u16,
        #[serde(default)]
        assertions: Vec<Assertion>,
    },
    Cli {
        command: CliCommand,
        #[serde(default)]
        assertions: Vec<Assertion>,
    },
    File {
        path: PathBuf,
        assertions: Vec<Assertion>,
    },
    Write {
        path: PathBuf,
        content: String,
    },
    Replace {
        path: PathBuf,
        old: String,
        new: String,
    },
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
}

impl HttpMethod {
    fn method(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CliCommand {
    Add,
    Fetch,
    Pull,
    Push,
    Status,
}

impl CliCommand {
    fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Fetch => "fetch",
            Self::Pull => "pull",
            Self::Push => "push",
            Self::Status => "status",
        }
    }

    fn args(self, case: &Case, id: &str) -> Vec<String> {
        let mut args = vec![
            self.name().into(),
            case.target.clone(),
            case.resource_type.clone(),
            id.into(),
            "--environment".into(),
            "live".into(),
        ];
        if let Some(namespace) = &case.namespace {
            args.extend(["--namespace".into(), namespace.clone()]);
        }
        match self {
            Self::Pull => args.extend(["--yes", "--missing", "conflict"].map(String::from)),
            Self::Push => args.extend(
                [
                    "--yes",
                    "--missing",
                    "restore",
                    "--uncommitted",
                    "allow",
                    "--untracked",
                    "allow",
                ]
                .map(String::from),
            ),
            _ => {}
        }
        args
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Assertion {
    Json { pointer: String, equals: Value },
    Contains { text: String },
    NotContains { text: String },
}

fn assertions_pass(assertions: &[Assertion], text: &str) -> Result<()> {
    let mut decoded = None;
    for assertion in assertions {
        match assertion {
            Assertion::Json { pointer, equals } => {
                if decoded.is_none() {
                    decoded =
                        Some(serde_json::from_str::<Value>(text).context("output is not JSON")?);
                }
                let actual = decoded.as_ref().unwrap().pointer(pointer);
                ensure!(
                    actual == Some(equals),
                    "JSON assertion at {pointer:?}: expected {equals}, got {actual:?}"
                );
            }
            Assertion::Contains { text: expected } => {
                ensure!(
                    text.contains(expected),
                    "contains assertion failed: {expected:?}"
                );
            }
            Assertion::NotContains { text: forbidden } => {
                ensure!(
                    !text.contains(forbidden),
                    "not_contains assertion failed: {forbidden:?}"
                );
            }
        }
    }
    Ok(())
}

fn safe_component(text: &str) -> Result<()> {
    ensure!(
        text.as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "expected a safe ASCII component (alphanumeric start, then alphanumeric, '.', '_', '-')"
    );
    Ok(())
}

fn file_path(path: &Path) -> Result<()> {
    ensure!(!path.as_os_str().is_empty(), "file path is empty");
    ensure!(
        path.components()
            .all(|part| matches!(part, Component::Normal(_))),
        "file path must be relative without '.' or '..'"
    );
    ensure!(
        !path
            .components()
            .any(|part| part.as_os_str().to_str().is_some_and(|name| {
                name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(".taku")
            })),
        "file steps cannot access .git or .taku"
    );
    Ok(())
}

fn local_path(project: &Path, relative: &Path) -> Result<PathBuf> {
    file_path(relative)?;
    let mut path = project.to_owned();
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "file path traverses a symlink"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

fn has_marker(path: &str, body: &Option<Value>, markers: &[&str]) -> bool {
    let body = body.as_ref().map(Value::to_string).unwrap_or_default();
    markers
        .iter()
        .any(|marker| path.contains(marker) || body.contains(marker))
}

fn validate_case(case: &Case) -> Result<()> {
    safe_component(&case.name)?;
    safe_component(&case.target)?;
    safe_component(&case.resource_type)?;
    ensure!(
        matches!(case.steps.first(), Some(Step::Http {
        method: HttpMethod::Get | HttpMethod::Post, path, body, status: 404, ..
    }) if has_marker(path, body, &["{{id}}"])),
        "first step must probe this case's {{{{id}}}} with GET/POST expecting 404"
    );
    ensure!(
        case.steps
            .iter()
            .any(|step| matches!(step, Step::Cli { .. })),
        "each case requires at least one scoped CLI step"
    );
    for (index, step) in case.steps.iter().enumerate() {
        match step {
            Step::Http {
                method,
                path,
                body,
                status,
                ..
            } => {
                ensure!((100..=599).contains(status), "invalid HTTP status");
                if !matches!(method, HttpMethod::Get) {
                    ensure!(
                        has_marker(path, body, &["{{id}}", "{{run}}"]),
                        "mutating HTTP step needs {{{{id}}}} or {{{{run}}}} in path/body"
                    );
                }
            }
            Step::Cli {
                command: CliCommand::Push,
                ..
            } => {
                ensure!(
                    matches!(case.steps.get(index + 1), Some(Step::Http {
                    method: HttpMethod::Get | HttpMethod::Post, path, body, status: 200..=299, assertions
                }) if !assertions.is_empty() && has_marker(path, body, &["{{id}}"])),
                    "push must be immediately followed by GET/POST 2xx independent readback with assertions and {{{{id}}}} in path/body"
                );
            }
            Step::File { path, .. } | Step::Write { path, .. } => file_path(path)?,
            Step::Replace { path, old, .. } => {
                file_path(path)?;
                ensure!(!old.is_empty(), "replace.old cannot be empty");
            }
            _ => {}
        }
    }
    Ok(())
}

// Render parsed scalar values and keys, never YAML source. Key collisions must not
// silently discard fixture data (including collisions inside JSON request bodies).
fn render(value: Value, run: &str, id: &str) -> Result<Value> {
    let substitute = |text: &str| text.replace("{{run}}", run).replace("{{id}}", id);
    Ok(match value {
        Value::String(text) => Value::String(substitute(&text)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| render(v, run, id))
                .collect::<Result<_>>()?,
        ),
        Value::Object(fields) => {
            let mut output = serde_json::Map::new();
            for (key, value) in fields {
                ensure!(
                    output
                        .insert(substitute(&key), render(value, run, id)?)
                        .is_none(),
                    "template expansion creates duplicate JSON keys"
                );
            }
            Value::Object(output)
        }
        other => other,
    })
}

#[derive(Default)]
struct Redactor(Vec<String>);

impl Redactor {
    fn add(&mut self, value: &str) {
        if !value.is_empty() {
            self.0.push(value.to_owned());
            // Reports escape JSON strings; also catch secrets returned as JSON strings.
            let escaped = serde_json::to_string(value).unwrap();
            self.0.push(escaped[1..escaped.len() - 1].to_owned());
            self.0.sort_by_key(|value| std::cmp::Reverse(value.len()));
            self.0.dedup();
        }
    }

    fn redact(&self, text: &str) -> String {
        self.0.iter().fold(text.to_owned(), |text, secret| {
            text.replace(secret, "[REDACTED]")
        })
    }

    fn check(&self, text: &str) -> Result<()> {
        ensure!(
            !self.0.iter().any(|secret| text.contains(secret)),
            "known environment value found in persistent content"
        );
        Ok(())
    }
}

// A CLI can persist an API response before this harness sees it. Scrub known
// environment values after each child exits and fail the step if that happened.
// This is best-effort literal/JSON-escaped matching, not arbitrary secret discovery.
fn scrub_project(directory: &Path, redactor: &Redactor) -> Result<bool> {
    let mut leaked = false;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "unexpected symlink in preserved project"
        );
        if kind.is_dir() {
            leaked |= scrub_project(&entry.path(), redactor)?;
        } else if kind.is_file() {
            let original = fs::read(entry.path())?;
            let mut bytes = original.clone();
            for secret in &redactor.0 {
                let mut remainder = bytes.as_slice();
                let mut clean = Vec::new();
                while let Some(index) = remainder
                    .windows(secret.len())
                    .position(|part| part == secret.as_bytes())
                {
                    clean.extend_from_slice(&remainder[..index]);
                    clean.extend_from_slice(b"[REDACTED]");
                    remainder = &remainder[index + secret.len()..];
                }
                clean.extend_from_slice(remainder);
                bytes = clean;
            }
            if bytes != original {
                fs::write(entry.path(), bytes)?;
                leaked = true;
            }
        }
    }
    Ok(leaked)
}

struct ReadyTarget {
    url: reqwest::Url,
    headers: HeaderMap,
    config: Value,
    application: String,
    files: BTreeMap<String, String>,
    resource_types: BTreeSet<String>,
    evidence: Value,
}

// Only this resolved form can execute: all fixtures, environment values, request
// URLs, headers, and application catalogs are checked before any remote request.
struct ReadySuite {
    run: String,
    targets: BTreeMap<String, ReadyTarget>,
    cases: Vec<Case>,
    environment: BTreeMap<String, String>,
    redactor: Redactor,
    client: Client,
}

fn environment_value(name: &str, env: &impl Fn(&str) -> Option<String>) -> Result<String> {
    ensure!(
        name.as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "invalid environment variable name"
    );
    env(name)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("required environment variable {name} is missing or empty"))
}

fn request_url(base: &reqwest::Url, path: &str) -> Result<reqwest::Url> {
    ensure!(
        path.starts_with('/')
            && !path.starts_with("//")
            && !path.contains(['\\', '#'])
            && !path.chars().any(char::is_control),
        "HTTP path must be an absolute-path reference, not a URL"
    );
    // URL parsers normalize literal and percent-encoded dot segments, which can
    // erase the ID-bearing segment or escape a target's base-path prefix.
    ensure!(
        !path.split('?').next().unwrap().split('/').any(|segment| {
            matches!(
                segment.to_ascii_lowercase().replace("%2e", ".").as_str(),
                "." | ".."
            )
        }),
        "HTTP path cannot contain dot segments"
    );
    let url = reqwest::Url::parse(&format!("{}{}", base.as_str().trim_end_matches('/'), path))
        .map_err(|_| anyhow::anyhow!("invalid HTTP path"))?;
    ensure!(url.origin() == base.origin(), "HTTP path changes origin");
    Ok(url)
}

impl ReadySuite {
    fn prepare(
        text: &str,
        directory: &Path,
        run: String,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        safe_component(&run)?;
        let suite: Suite = serde_yaml::from_str(text)
            .map_err(|error| anyhow::anyhow!("invalid suite schema at {:?}", error.location()))?;
        ensure!(
            !suite.targets.is_empty() && !suite.cases.is_empty(),
            "suite needs targets and cases"
        );
        let mut environment = BTreeMap::new();
        let mut redactor = Redactor::default();
        for target in suite.targets.values() {
            for name in std::iter::once(&target.url_env).chain(target.authorization_env.iter()) {
                let value = environment_value(name, &env)?;
                redactor.add(&value);
                if target.authorization_env.as_ref() == Some(name)
                    && let Some((_, token)) = value.split_once(char::is_whitespace)
                {
                    redactor.add(token.trim());
                }
                environment.insert(name.clone(), value);
            }
        }
        let prepared = (|| {
            redactor.check(text)?;
            let mut targets = BTreeMap::new();
            let mut applications = BTreeMap::new();
            for (name, target) in suite.targets {
                safe_component(&name)?;
                let definition = serde_json::to_string(&target)?;
                ensure!(
                    !definition.contains("{{run}}") && !definition.contains("{{id}}"),
                    "target definitions cannot contain case templates"
                );
                let url = reqwest::Url::parse(&environment[&target.url_env])
                    .map_err(|_| anyhow::anyhow!("invalid URL in {}", target.url_env))?;
                ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none(),
                    "target URL must be HTTP(S), without credentials, query, or fragment"
                );
                let mut headers = HeaderMap::new();
                for (name, value) in &target.headers {
                    let name =
                        HeaderName::from_bytes(name.as_bytes()).context("invalid header name")?;
                    ensure!(
                        !matches!(
                            name.as_str(),
                            "authorization" | "proxy-authorization" | "cookie" | "host"
                        ),
                        "use authorization_env for credentials; host/cookie overrides are not supported"
                    );
                    headers.insert(
                        name,
                        HeaderValue::from_str(value).context("invalid header value")?,
                    );
                }
                let mut fields = BTreeMap::from([("url", target.url_env.clone())]);
                if let Some(variable) = &target.authorization_env {
                    let mut value = HeaderValue::from_str(&environment[variable])
                        .context("invalid authorization header")?;
                    value.set_sensitive(true);
                    headers.insert(AUTHORIZATION, value);
                    fields.insert("authorization", variable.clone());
                }
                ensure!(
                    !target.application.is_absolute()
                        && target
                            .application
                            .file_name()
                            .is_some_and(|n| n == "application.yaml"),
                    "application must be a suite-relative application.yaml path"
                );
                let source = directory.join(&target.application);
                let application_text =
                    fs::read_to_string(&source).context("cannot read application.yaml")?;
                redactor.check(&application_text)?;
                let application: resource_control::ApplicationDefinition =
                    serde_yaml::from_str(&application_text)
                        .context("invalid application definition")?;
                let application_name = application.application.name;
                safe_component(&application_name)?;
                let mut files = BTreeMap::from([("application.yaml".into(), application_text)]);
                let mut resource_types = BTreeSet::new();
                let mut catalogs = BTreeMap::new();
                for entry in fs::read_dir(source.parent().context("application has no directory")?)?
                {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with("version-") && name.ends_with(".yaml") {
                        let text = fs::read_to_string(entry.path())?;
                        redactor.check(&text)?;
                        let catalog: resource_control::ResourceTypeCatalog =
                            serde_yaml::from_str(&text).context("invalid version catalog")?;
                        ensure!(
                            catalog.application.name == application_name,
                            "catalog belongs to another application"
                        );
                        catalogs.insert(
                            name.clone(),
                            json!({
                                "definition_version": catalog.version,
                                "application_version": catalog.application.version,
                            }),
                        );
                        resource_types.extend(catalog.resource_types.into_keys());
                        files.insert(name, text);
                    }
                }
                ensure!(
                    files.len() > 1,
                    "application has no sibling version-*.yaml catalogs"
                );
                if let Some(previous) = applications.insert(application_name.clone(), files.clone())
                {
                    ensure!(
                        previous == files,
                        "conflicting catalogs for the same application name"
                    );
                }
                let evidence = json!({
                    "application": application_name,
                    "application_path": target.application,
                    "definition_version": application.version,
                    "catalogs": catalogs,
                    "url_env": target.url_env,
                    "authorization_env": target.authorization_env,
                });
                let config = json!({
                    "application": application_name,
                    "url": "http://taku-live.invalid",
                    "auth": {"fields": fields},
                    "headers": target.headers,
                });
                targets.insert(
                    name,
                    ReadyTarget {
                        url,
                        headers,
                        config,
                        application: application_name,
                        files,
                        resource_types,
                        evidence,
                    },
                );
            }
            let mut cases = Vec::new();
            let mut names = BTreeSet::new();
            for case in suite.cases {
                validate_case(&case).with_context(|| format!("case {}", case.name))?;
                ensure!(names.insert(case.name.clone()), "duplicate case name");
                let id = format!("{run}-{}", case.name);
                let case: Case =
                    serde_json::from_value(render(serde_json::to_value(case)?, &run, &id)?)?;
                safe_component(&case.target)?;
                safe_component(&case.resource_type)?;
                if let Some(namespace) = &case.namespace {
                    safe_component(namespace)?;
                }
                let target = targets
                    .get(&case.target)
                    .context("case references an unknown target")?;
                ensure!(
                    target.resource_types.contains(&case.resource_type),
                    "unknown resource type in case {}",
                    case.name
                );
                for step in &case.steps {
                    match step {
                        Step::Http { path, .. } => {
                            request_url(&target.url, path)?;
                        }
                        Step::File { path, .. }
                        | Step::Write { path, .. }
                        | Step::Replace { path, .. } => file_path(path)?,
                        _ => {}
                    }
                    let assertions = match step {
                        Step::Http { assertions, .. }
                        | Step::Cli { assertions, .. }
                        | Step::File { assertions, .. } => assertions.as_slice(),
                        _ => &[],
                    };
                    for assertion in assertions {
                        if let Assertion::Json { pointer, .. } = assertion {
                            ensure!(valid_pointer(pointer), "invalid JSON pointer");
                        }
                    }
                }
                cases.push(case);
            }
            Ok((targets, cases))
        })();
        let (targets, cases) = prepared.map_err(|error: anyhow::Error| {
            anyhow::anyhow!(redactor.redact(&format!("{error:#}")))
        })?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .build()?;
        Ok(Self {
            run,
            targets,
            cases,
            environment,
            redactor,
            client,
        })
    }

    fn cli(&self, project: &Path, args: &[String], observation: &mut Observation) -> Result<()> {
        let mut command = Command::cargo_bin("taku")?;
        command
            .current_dir(project)
            .envs(&self.environment)
            .timeout(TIMEOUT)
            .args(["--non-interactive", "--output", "json"])
            .args(args);
        let result = capture(&mut command, observation);
        let leaked = scrub_project(project, &self.redactor)?;
        ensure!(
            !leaked,
            "CLI persisted a known environment value; preserved files were redacted"
        );
        result
    }

    fn initialize(&self, project: &Path, report: &mut Value) -> Result<()> {
        fs::create_dir(project)?;
        let mut observation = Observation::default();
        let result = capture(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(project)
                .timeout(TIMEOUT),
            &mut observation,
        );
        report["setup"]
            .as_array_mut()
            .unwrap()
            .push(json!({"command": "git init", "output": observation}));
        result?;
        let mut observation = Observation::default();
        let result = self.cli(
            project,
            &["init", "--layout", "single", "--environment", "live"].map(String::from),
            &mut observation,
        );
        report["setup"]
            .as_array_mut()
            .unwrap()
            .push(json!({"command": "taku init", "output": observation}));
        result?;
        for target in self.targets.values() {
            let directory = project.join(".taku/applications").join(&target.application);
            fs::create_dir_all(&directory)?;
            for (name, text) in &target.files {
                fs::write(directory.join(name), text)?;
            }
        }
        let path = project.join(".taku/project.yaml");
        let mut config: Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
        config["environments"]["live"]["targets"] = self
            .targets
            .iter()
            .map(|(name, target)| (name.clone(), target.config.clone()))
            .collect();
        let text = serde_yaml::to_string(&config)?;
        self.redactor.check(&text)?;
        fs::write(path, text)?;
        // app is local-only and uses the real catalog parser. Unlike validate, it
        // does not require remote version baselines for multi-catalog applications.
        let mut observation = Observation::default();
        let result = self.cli(project, &["app".into()], &mut observation);
        report["setup"]
            .as_array_mut()
            .unwrap()
            .push(json!({"command": "taku app", "output": observation}));
        result
    }

    fn execute(
        &self,
        project: &Path,
        case: &Case,
        step: &Step,
        output: &mut Observation,
    ) -> Result<()> {
        match step {
            Step::Http {
                method,
                path,
                body,
                status,
                assertions,
            } => {
                let target = &self.targets[&case.target];
                let mut request = self
                    .client
                    .request(method.method(), request_url(&target.url, path)?)
                    .headers(target.headers.clone());
                if let Some(body) = body {
                    request = request.json(body);
                }
                let response = request.send().context("HTTP request failed")?;
                output.http_status = Some(response.status().as_u16());
                output.stdout = response.text().context("cannot read HTTP response")?;
                ensure!(
                    output.http_status == Some(*status),
                    "expected HTTP {status}, got {:?}",
                    output.http_status
                );
                assertions_pass(assertions, &output.stdout)
            }
            Step::Cli {
                command,
                assertions,
            } => {
                let id = format!("{}-{}", self.run, case.name);
                self.cli(project, &command.args(case, &id), output)?;
                assertions_pass(assertions, &output.stdout)
            }
            Step::File { path, assertions } => {
                output.stdout = fs::read_to_string(local_path(project, path)?)?;
                assertions_pass(assertions, &output.stdout)
            }
            Step::Write { path, content } => {
                self.redactor.check(content)?;
                let path = local_path(project, path)?;
                fs::create_dir_all(path.parent().context("file has no parent")?)?;
                fs::write(path, content)?;
                Ok(())
            }
            Step::Replace { path, old, new } => {
                let path = local_path(project, path)?;
                let text = fs::read_to_string(&path)?;
                ensure!(
                    text.find(old).is_some() && text.find(old) == text.rfind(old),
                    "replace requires exactly 1 occurrence"
                );
                let text = text.replacen(old, new, 1);
                self.redactor.check(&text)?;
                fs::write(path, text)?;
                Ok(())
            }
        }
    }

    fn run(&self, root: &Path) -> Result<PathBuf> {
        ensure!(root.is_dir(), "artifact root must already exist");
        let directory = root.join(&self.run);
        fs::create_dir(&directory).context("cannot create unique artifact directory")?;
        let project = directory.join("project");
        let report_path = directory.join("report.json");
        let cases: Vec<Value> = self
            .cases
            .iter()
            .map(|case| {
                json!({
                    "name": case.name, "target": case.target,
                    "application": self.targets[&case.target].application,
                    "resource_type": case.resource_type, "namespace": case.namespace,
                    "id": format!("{}-{}", self.run, case.name),
                    "status": "pending",
                    "steps": case.steps.iter().map(|step| json!({
                        "kind": serde_json::to_value(step).unwrap()["kind"],
                        "spec": step, "status": "pending"
                    })).collect::<Vec<_>>()
                })
            })
            .collect();
        let targets: BTreeMap<_, _> = self
            .targets
            .iter()
            .map(|(name, target)| (name, &target.evidence))
            .collect();
        let mut report = json!({
            "run": self.run, "status": "initializing", "setup": [],
            "targets": targets, "cases": cases,
        });
        self.save(&report_path, &report)?;
        if let Err(error) = self.initialize(&project, &mut report) {
            report["status"] = json!("failed");
            report["error"] = json!(format!("{error:#}"));
            self.save(&report_path, &report)?;
            bail!(
                "live suite initialization failed; see {}",
                report_path.display()
            );
        }
        report["status"] = json!("running");
        self.save(&report_path, &report)?;
        let mut failed = false;
        for (case_index, case) in self.cases.iter().enumerate() {
            let mut case_failed = false;
            for (step_index, step) in case.steps.iter().enumerate() {
                if case_failed {
                    report["cases"][case_index]["steps"][step_index]["status"] = json!("skipped");
                    continue;
                }
                report["cases"][case_index]["status"] = json!("running");
                report["cases"][case_index]["steps"][step_index]["status"] = json!("running");
                self.save(&report_path, &report)?;
                let mut output = Observation::default();
                let result = self.execute(&project, case, step, &mut output);
                let entry = &mut report["cases"][case_index]["steps"][step_index];
                entry["output"] = serde_json::to_value(output)?;
                match result {
                    Ok(()) => entry["status"] = json!("passed"),
                    Err(error) => {
                        entry["status"] = json!("failed");
                        entry["error"] = json!(format!("{error:#}"));
                        case_failed = true;
                    }
                }
                self.save(&report_path, &report)?;
            }
            failed |= case_failed;
            report["cases"][case_index]["status"] =
                json!(if case_failed { "failed" } else { "passed" });
            self.save(&report_path, &report)?;
        }
        report["status"] = json!(if failed { "failed" } else { "passed" });
        self.save(&report_path, &report)?;
        ensure!(!failed, "live suite failed; see {}", report_path.display());
        Ok(directory)
    }

    fn save(&self, path: &Path, report: &Value) -> Result<()> {
        // Redact values before encoding, so quotes/backslashes in secrets cannot
        // corrupt report JSON. Never serialize request headers or environment maps.
        fn scrub(value: &mut Value, redactor: &Redactor) -> Result<()> {
            match value {
                Value::String(text) => *text = redactor.redact(text),
                Value::Array(values) => {
                    for value in values {
                        scrub(value, redactor)?;
                    }
                }
                Value::Object(values) => {
                    for (key, mut value) in std::mem::take(values) {
                        scrub(&mut value, redactor)?;
                        ensure!(
                            values.insert(redactor.redact(&key), value).is_none(),
                            "redaction would discard colliding report keys"
                        );
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut report = report.clone();
        scrub(&mut report, &self.redactor)?;
        let text = serde_json::to_string_pretty(&report)?;
        self.redactor.check(&text)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text)?;
        fs::rename(temporary, path)?;
        Ok(())
    }
}

fn valid_pointer(pointer: &str) -> bool {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(character) = chars.next() {
        if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

#[derive(Default, Serialize)]
struct Observation {
    http_status: Option<u16>,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn capture(command: &mut Command, observation: &mut Observation) -> Result<()> {
    // Both git init and Taku's Git children must stay in the generated project,
    // even when this test is launched from a Git hook or custom worktree setup.
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
    ] {
        command.env_remove(variable);
    }
    // assert_cmd's output() honors timeout(), kills/reaps the child, and keeps
    // captured output. A timeout/signal has no successful exit code.
    let output = command.output().context("cannot run subprocess")?;
    observation.exit_code = output.status.code();
    observation.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    observation.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    ensure!(
        output.status.success(),
        "subprocess failed or timed out (exit {:?})",
        observation.exit_code
    );
    Ok(())
}

fn run_id() -> Result<String> {
    Ok(format!(
        "taku-live-{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::process::id()
    ))
}

fn live_settings(env: impl Fn(&str) -> Option<String>) -> Result<(PathBuf, PathBuf)> {
    ensure!(
        env("TAKU_LIVE_ALLOW_MUTATIONS").as_deref() == Some("1"),
        "TAKU_LIVE_ALLOW_MUTATIONS=1 is required"
    );
    let suite = PathBuf::from(environment_value("TAKU_LIVE_SUITE", &env)?);
    let artifacts = PathBuf::from(environment_value("TAKU_LIVE_ARTIFACTS", &env)?);
    ensure!(suite.is_file(), "TAKU_LIVE_SUITE must name a YAML file");
    ensure!(
        artifacts.is_dir(),
        "TAKU_LIVE_ARTIFACTS must name an existing directory"
    );
    Ok((suite, artifacts))
}

#[test]
#[ignore = "explicit opt-in only: TAKU_LIVE_SUITE, TAKU_LIVE_ARTIFACTS, TAKU_LIVE_ALLOW_MUTATIONS=1"]
fn live_api_suite() {
    let env = |name: &str| std::env::var(name).ok();
    let (suite, artifacts) = live_settings(env).expect("live suite opt-in settings");
    let text = fs::read_to_string(&suite).expect("read suite YAML");
    let ready = ReadySuite::prepare(&text, suite.parent().unwrap(), run_id().unwrap(), env)
        .unwrap_or_else(|error| panic!("live suite preflight: {error:#}"));
    let directory = ready
        .run(&artifacts)
        .unwrap_or_else(|error| panic!("{}", ready.redactor.redact(&format!("{error:#}"))));
    eprintln!(
        "Live API suite passed; retained report: {}",
        directory.join("report.json").display()
    );
}
