use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use tempfile::TempDir;
use tiny_http::{Response, Server};

#[derive(Clone, Copy)]
enum Behavior {
    Normal,
    SemanticFailure,
    WrongReadback,
}

struct MockApi {
    url: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockApi {
    fn start(behavior: Behavior) -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, stopping) = (requests.clone(), stop.clone());
        let thread = thread::spawn(move || {
            let mut resources = BTreeMap::<String, Value>::new();
            while !stopping.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(10)).unwrap()
                else {
                    continue;
                };
                let method = request.method().as_str().to_owned();
                let path = request.url().to_owned();
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push((method.clone(), path.clone()));
                let (status, value) = match (method.as_str(), path.as_str()) {
                    ("GET", "/version") => (200, json!({"version": "1.0.0"})),
                    ("GET", "/widgets") => (200, json!(resources.values().collect::<Vec<_>>())),
                    ("GET", "/redirect") => {
                        request
                            .respond(Response::from_string("").with_status_code(302).with_header(
                                tiny_http::Header::from_bytes("Location", "/version").unwrap(),
                            ))
                            .unwrap();
                        continue;
                    }
                    ("PUT", path) if path.starts_with("/widgets/") => {
                        let mut value: Value = serde_json::from_str(&body).unwrap();
                        let id = path.trim_start_matches("/widgets/").to_owned();
                        value["id"] = json!(id);
                        let existed = resources.contains_key(&id);
                        if !matches!(behavior, Behavior::WrongReadback) || !existed {
                            resources.insert(id, value);
                        }
                        (
                            200,
                            json!({"accepted": !matches!(behavior, Behavior::SemanticFailure)}),
                        )
                    }
                    ("GET", path) if path.starts_with("/widgets/") => {
                        match resources.get(path.trim_start_matches("/widgets/")) {
                            Some(value) => (200, value.clone()),
                            None => (404, json!({"missing": true})),
                        }
                    }
                    _ => (404, json!({"missing": true})),
                };
                request
                    .respond(
                        Response::from_string(value.to_string())
                            .with_status_code(status)
                            .with_header(
                                tiny_http::Header::from_bytes("Content-Type", "application/json")
                                    .unwrap(),
                            ),
                    )
                    .unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for MockApi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn catalogs(directory: &Path) {
    fs::create_dir(directory.join("catalog")).unwrap();
    fs::write(
        directory.join("catalog/application.yaml"),
        r#"schema_version: 1
version: "1.0.0"
application: { name: mock }
target_profile: { headers: {} }
version_endpoints:
  - { method: GET, path: /version, pointer: /version }
"#,
    )
    .unwrap();
    fs::write(
        directory.join("catalog/version-1.yaml"),
        r#"schema_version: 1
version: "1.0.0"
application: { name: mock, version: ">=1.0.0, <2.0.0" }
resource_types:
  widgets:
    - id: { pointer: /id, scope: universal }
      display_name: { strategy: id }
      operations:
        read: { method: GET, path: "/widgets/{id}" }
        list: { method: GET, path: /widgets, cardinality: many, response: { collection: list } }
        upsert: { method: PUT, path: "/widgets/{id}" }
"#,
    )
    .unwrap();
}

fn fixture() -> Value {
    json!({
        "targets": {"api": {"application": "catalog/application.yaml", "url_env": "MOCK_URL", "authorization_env": "MOCK_AUTH", "headers": {"x-live-test": "true"}}},
        "cases": [{"name": "widget", "target": "api", "resource_type": "widgets", "steps": [
            {"kind": "http", "method": "GET", "path": "/widgets/{{id}}", "status": 404},
            {"kind": "http", "method": "PUT", "path": "/widgets/{{id}}", "body": {"id": "{{id}}", "name": "before"}, "status": 200,
             "assertions": [{"kind": "json", "pointer": "/accepted", "equals": true}]},
            {"kind": "cli", "command": "add"},
            {"kind": "cli", "command": "fetch"},
            {"kind": "cli", "command": "pull"},
            {"kind": "file", "path": "api/widgets/{{id}}.json", "assertions": [{"kind": "json", "pointer": "/name", "equals": "before"}]},
            {"kind": "replace", "path": "api/widgets/{{id}}.json", "old": "before", "new": "after"},
            {"kind": "cli", "command": "push", "assertions": [{"kind": "json", "pointer": "/result/0/outcome", "equals": "success"}]},
            {"kind": "http", "method": "GET", "path": "/widgets/{{id}}", "status": 200,
             "assertions": [{"kind": "json", "pointer": "/name", "equals": "after"}]},
            {"kind": "cli", "command": "fetch"},
            {"kind": "cli", "command": "status", "assertions": [{"kind": "contains", "text": "in_sync"}]}
        ]}]
    })
}

fn ready(fixture: &Value, directory: &Path, url: &str, run: &str) -> Result<ReadySuite> {
    ReadySuite::prepare(
        &serde_yaml::to_string(fixture).unwrap(),
        directory,
        run.into(),
        |key| match key {
            "MOCK_URL" => Some(url.into()),
            "MOCK_AUTH" => Some("Bearer mock-secret-token".into()),
            _ => None,
        },
    )
}

fn report(directory: &Path, run: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(directory.join(run).join("report.json")).unwrap())
        .unwrap()
}

#[test]
fn compiled_cli_and_independent_http_complete_the_declarative_workflow() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let api = MockApi::start(Behavior::Normal);
    let run = run_id().unwrap();
    let suite = ready(&fixture(), directory.path(), &api.url, &run).unwrap();
    let result = suite.run(directory.path());
    let report = report(directory.path(), &run);
    assert!(result.is_ok(), "{result:?}\n{report:#}");
    assert_eq!(report["status"], "passed");
    assert_eq!(report["cases"][0]["id"], format!("{run}-widget"));
    assert_eq!(report["cases"][0]["steps"][8]["output"]["http_status"], 200);
    let steps = &report["cases"][0]["steps"];
    for (index, step) in suite.cases[0].steps.iter().enumerate() {
        assert_eq!(steps[index]["spec"], serde_json::to_value(step).unwrap());
    }
    assert_eq!(steps[8]["spec"]["path"], format!("/widgets/{run}-widget"));
    assert_eq!(steps[7]["spec"]["command"], "push");
    assert_eq!(steps[8]["spec"]["assertions"][0]["equals"], "after");
    assert_eq!(
        report["targets"]["api"],
        json!({
            "application": "mock", "application_path": "catalog/application.yaml",
            "definition_version": "1.0.0",
            "catalogs": {"version-1.yaml": {
                "definition_version": "1.0.0", "application_version": ">=1.0.0, <2.0.0"
            }},
            "url_env": "MOCK_URL", "authorization_env": "MOCK_AUTH",
        })
    );
    let persisted = report.to_string();
    assert!(!persisted.contains(&api.url) && !persisted.contains("mock-secret-token"));
    let project = directory.path().join(&run).join("project");
    let config = fs::read_to_string(project.join(".taku/project.yaml")).unwrap();
    assert!(config.contains("MOCK_URL") && config.contains("MOCK_AUTH"));
    assert!(!config.contains(&api.url) && !config.contains("mock-secret-token"));
    for file in ["application.yaml", "version-1.yaml"] {
        assert_eq!(
            fs::read(project.join(".taku/applications/mock").join(file)).unwrap(),
            fs::read(directory.path().join("catalog").join(file)).unwrap()
        );
    }
    let requests = api.requests.lock().unwrap();
    assert_eq!(
        requests.first().unwrap(),
        &("GET".into(), format!("/widgets/{run}-widget"))
    );
    assert!(!requests.iter().any(|(method, _)| method == "DELETE"));
    assert!(
        requests
            .iter()
            .filter(|(method, _)| method == "PUT")
            .count()
            >= 2
    );
}

#[test]
fn http_200_semantic_failure_stops_case_but_runs_other_cases() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let api = MockApi::start(Behavior::SemanticFailure);
    let mut fixture = fixture();
    fixture["cases"].as_array_mut().unwrap().push(json!({
        "name": "other", "target": "api", "resource_type": "widgets", "steps": [
            {"kind": "http", "method": "GET", "path": "/widgets/{{id}}", "status": 404},
            {"kind": "http", "method": "PUT", "path": "/widgets/{{id}}", "status": 200, "body": {"id": "{{id}}"}},
            {"kind": "write", "path": "{{id}}.txt", "content": "still runs"},
            {"kind": "file", "path": "{{id}}.txt", "assertions": [{"kind": "contains", "text": "still runs"}]},
            {"kind": "cli", "command": "add"}
        ]
    }));
    let suite = ready(&fixture, directory.path(), &api.url, "taku-live-semantic").unwrap();
    assert!(suite.run(directory.path()).is_err());
    let report = report(directory.path(), &suite.run);
    assert_eq!(report["cases"][0]["steps"][1]["output"]["http_status"], 200);
    assert_eq!(report["cases"][0]["steps"][1]["status"], "failed");
    assert_eq!(report["cases"][0]["steps"][2]["status"], "skipped");
    assert_eq!(report["cases"][1]["status"], "passed");
    assert_eq!(
        api.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, path)| path.ends_with("-widget"))
            .count(),
        2
    );
}

#[test]
fn successful_push_cannot_hide_wrong_independent_readback() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let api = MockApi::start(Behavior::WrongReadback);
    let suite = ready(
        &fixture(),
        directory.path(),
        &api.url,
        "taku-live-wrong-readback",
    )
    .unwrap();
    assert!(suite.run(directory.path()).is_err());
    let report = report(directory.path(), &suite.run);
    assert_eq!(
        report["cases"][0]["steps"][7]["status"], "passed",
        "{report:#}"
    );
    assert_eq!(report["cases"][0]["steps"][7]["output"]["exit_code"], 0);
    assert_eq!(report["cases"][0]["steps"][8]["status"], "failed");
    assert_eq!(report["cases"][0]["steps"][9]["status"], "skipped");
    assert_eq!(
        report["cases"][0]["steps"][8]["spec"]["path"],
        "/widgets/taku-live-wrong-readback-widget"
    );
    assert_eq!(
        report["cases"][0]["steps"][8]["spec"]["assertions"][0]["equals"],
        "after"
    );
    assert_eq!(report["cases"][0]["steps"][9]["spec"]["command"], "fetch");
}

#[test]
fn pull_conflict_preserves_desired_file_despite_catalog_missing_delete() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let catalog_path = directory.path().join("catalog/version-1.yaml");
    let mut catalog: Value =
        serde_yaml::from_str(&fs::read_to_string(&catalog_path).unwrap()).unwrap();
    catalog["resource_types"]["widgets"][0]["missing"] = json!({"pull": "delete"});
    fs::write(&catalog_path, serde_yaml::to_string(&catalog).unwrap()).unwrap();

    let api = MockApi::start(Behavior::Normal);
    let mut fixture = fixture();
    let desired = json!({"_taku": {"id": "{{id}}"}, "id": "{{id}}", "name": "keep me"});
    fixture["cases"][0]["steps"] = json!([
        {"kind": "http", "method": "GET", "path": "/widgets/{{id}}", "status": 404},
        {"kind": "write", "path": "api/widgets/{{id}}.json", "content": desired.to_string()},
        {"kind": "cli", "command": "fetch", "assertions": [
            {"kind": "json", "pointer": "/result/0/outcome", "equals": "absent"}
        ]},
        {"kind": "cli", "command": "pull"},
        {"kind": "file", "path": "api/widgets/{{id}}.json", "assertions": [
            {"kind": "json", "pointer": "/name", "equals": "keep me"}
        ]}
    ]);
    let run = "taku-live-pull-conflict";
    let suite = ready(&fixture, directory.path(), &api.url, run).unwrap();
    let result = suite.run(directory.path());
    let report = report(directory.path(), run);
    assert!(
        result.is_err(),
        "pull failure must fail the suite: {report:#}"
    );
    assert_eq!(report["status"], "failed");
    assert_eq!(report["cases"][0]["status"], "failed");
    let steps = &report["cases"][0]["steps"];
    assert_eq!(steps[0]["output"]["http_status"], 404);
    assert_eq!(steps[2]["status"], "passed", "{report:#}");
    assert_eq!(steps[3]["status"], "failed", "{report:#}");
    assert!(
        steps[3]["output"]["exit_code"]
            .as_i64()
            .is_some_and(|code| code != 0)
    );
    assert_eq!(steps[4]["status"], "skipped");

    let path = directory
        .path()
        .join(run)
        .join("project/api/widgets")
        .join(format!("{run}-widget.json"));
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        desired
            .to_string()
            .replace("{{id}}", &format!("{run}-widget"))
    );
    let requests = api.requests.lock().unwrap();
    assert!(
        requests.iter().all(|(method, _)| method == "GET"),
        "no seed or DELETE: {requests:?}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|(_, path)| path == &format!("/widgets/{run}-widget"))
            .count(),
        2
    );
}

#[test]
fn safety_parsing_rejects_bad_suites_before_network_access() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let api = MockApi::start(Behavior::Normal);
    let variants = [
        ("/cases/0/name", json!("../escape")),
        ("/cases/0/name", json!("*")),
        ("/cases/0/target", json!("missing")),
        ("/cases/0/resource_type", json!("*")),
        ("/cases/0/namespace", json!("--all")),
        ("/cases/0/steps/0/method", json!("DELETE")),
        ("/cases/0/steps/0/status", json!(200)),
        ("/cases/0/steps/0/path", json!("/widgets/{{run}}")),
        ("/cases/0/steps/1/path", json!("/widgets")),
        ("/cases/0/steps/2/command", json!("remove")),
        ("/cases/0/steps/5/path", json!("../escape")),
        ("/cases/0/steps/5/path", json!("/absolute")),
        ("/cases/0/steps/5/path", json!(".taku/project.yaml")),
        ("/cases/0/steps/5/path", json!(".TAKU/project.yaml")),
        ("/cases/0/steps/5/path", json!(".Git/config")),
        ("/cases/0/steps/6/old", json!("")),
        ("/cases/0/steps/8/assertions", json!([])),
        ("/cases/0/steps/8/method", json!("PUT")),
        ("/cases/0/steps/8/status", json!(404)),
        ("/cases/0/steps/8/path", json!("/health")),
        ("/cases/0/steps/8/path", json!("/widgets/{{run}}")),
        ("/cases/0/steps/8/path", json!("/widgets/{{id}}/../health")),
        (
            "/cases/0/steps/8/path",
            json!("/widgets/{{id}}/%2E%2e/health"),
        ),
        (
            "/cases/0/steps/8/path",
            json!("https://other.invalid/{{id}}"),
        ),
        ("/cases/0/steps/8/path", json!("//other.invalid/{{id}}")),
        (
            "/cases/0/steps/8/assertions/0/pointer",
            json!("/bad~2escape"),
        ),
        (
            "/targets/api/application",
            json!("/absolute/application.yaml"),
        ),
        ("/targets/api/url_env", json!("MISSING")),
    ];
    for (pointer, value) in variants {
        let mut fixture = fixture();
        // namespace is optional and not present in the base fixture.
        fixture["cases"][0]["namespace"] = Value::Null;
        *fixture.pointer_mut(pointer).unwrap() = value;
        if pointer == "/cases/0/steps/1/path" {
            fixture["cases"][0]["steps"][1]["body"] = Value::Null;
        }
        assert!(
            ready(&fixture, directory.path(), &api.url, "taku-live-parse").is_err(),
            "accepted {pointer}: {fixture}"
        );
    }
    let mut typo = fixture();
    typo["cases"][0]["steps"][0]["assertion"] = json!([]);
    assert!(ready(&typo, directory.path(), &api.url, "taku-live-parse").is_err());
    let mut duplicate = fixture();
    let case = duplicate["cases"][0].clone();
    duplicate["cases"].as_array_mut().unwrap().push(case);
    assert!(ready(&duplicate, directory.path(), &api.url, "taku-live-parse").is_err());
    let mut api_only = fixture();
    api_only["cases"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .retain(|step| step["kind"] != "cli");
    let error = ready(&api_only, directory.path(), &api.url, "taku-live-parse")
        .err()
        .unwrap();
    assert!(error.to_string().contains("scoped CLI step"));
    assert!(api.requests.lock().unwrap().is_empty());
}

#[test]
fn body_selected_readback_and_templated_namespace_keep_exact_cli_scope() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let mut fixture = fixture();
    fixture["cases"][0]["namespace"] = json!("{{run}}-space");
    fixture["cases"][0]["steps"][8]["method"] = json!("POST");
    fixture["cases"][0]["steps"][8]["path"] = json!("/lookup");
    fixture["cases"][0]["steps"][8]["body"] = json!({"ids": ["{{id}}"]});
    let suite = ready(
        &fixture,
        directory.path(),
        "http://127.0.0.1:1",
        "taku-live-scope",
    )
    .unwrap();
    let case = &suite.cases[0];
    assert_eq!(case.namespace.as_deref(), Some("taku-live-scope-space"));
    let args = CliCommand::Add.args(case, "taku-live-scope-widget");
    assert_eq!(
        args,
        [
            "add",
            "api",
            "widgets",
            "taku-live-scope-widget",
            "--environment",
            "live",
            "--namespace",
            "taku-live-scope-space"
        ]
    );
}

#[test]
fn shipped_suites_resolve_their_real_application_configs_without_network() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/live");
    let mut count = 0;
    for entry in fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_some_and(|extension| extension == "yaml")
        {
            let text = fs::read_to_string(&path).unwrap();
            let suite: Suite = serde_yaml::from_str(&text).unwrap();
            let environment: BTreeMap<_, _> =
                suite
                    .targets
                    .values()
                    .flat_map(|target| {
                        std::iter::once((target.url_env.clone(), "http://127.0.0.1:1".to_owned()))
                            .chain(target.authorization_env.iter().map(|name| {
                                (name.clone(), "Bearer fixture-test-secret".to_owned())
                            }))
                    })
                    .collect();
            ReadySuite::prepare(
                &text,
                &directory,
                "taku-live-fixture-check".into(),
                |name| environment.get(name).cloned(),
            )
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
            count += 1;
        }
    }
    assert!(count > 0, "no shipped YAML suites checked");
}

#[test]
fn templates_preserve_json_types_keys_and_escaped_scalar_content() {
    let value = json!({"{{id}}": [{"nested": "{{run}}\n\"quoted\"", "bool": true, "null": null, "number": 2}]});
    let rendered = render(value, "taku-live-run", "taku-live-run-case").unwrap();
    assert_eq!(
        rendered["taku-live-run-case"][0]["nested"],
        "taku-live-run\n\"quoted\""
    );
    assert_eq!(rendered["taku-live-run-case"][0]["bool"], true);
    assert_eq!(rendered["taku-live-run-case"][0]["number"], 2);
    assert!(rendered["taku-live-run-case"][0]["null"].is_null());
    assert!(render(json!({"{{id}}": 1, "literal": 2}), "run", "literal").is_err());
    let id = run_id().unwrap();
    assert!(id.starts_with("taku-live-"));
    safe_component(&id).unwrap();
}

#[test]
fn root_json_and_raw_text_assertions_have_distinct_semantics() {
    let root: Assertion =
        serde_json::from_value(json!({"kind": "json", "pointer": "", "equals": {"ok": true}}))
            .unwrap();
    assert!(assertions_pass(&[root], "{\"ok\":true}").is_ok());
    let missing: Assertion =
        serde_json::from_value(json!({"kind": "json", "pointer": "/absent", "equals": null}))
            .unwrap();
    assert!(assertions_pass(&[missing], "{}").is_err());
    assert!(
        assertions_pass(
            &[Assertion::Contains { text: "\\n".into() }],
            "\"line\\nbreak\""
        )
        .is_ok()
    );
    assert!(
        assertions_pass(
            &[Assertion::NotContains {
                text: "error".into()
            }],
            "error"
        )
        .is_err()
    );
    assert!(
        assertions_pass(
            &[Assertion::Json {
                pointer: "".into(),
                equals: Value::Null
            }],
            "not JSON"
        )
        .is_err()
    );
}

#[test]
fn file_replace_requires_one_match_and_root_json_checks_work() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let suite = ready(
        &fixture(),
        directory.path(),
        "http://127.0.0.1:1",
        "taku-live-files",
    )
    .unwrap();
    let case = &suite.cases[0];
    let path = PathBuf::from("nested/file.json");
    let mut output = Observation::default();
    suite
        .execute(
            directory.path(),
            case,
            &Step::Write {
                path: path.clone(),
                content: "{\"one\":1}".into(),
            },
            &mut output,
        )
        .unwrap();
    suite
        .execute(
            directory.path(),
            case,
            &Step::File {
                path: path.clone(),
                assertions: vec![Assertion::Json {
                    pointer: "".into(),
                    equals: json!({"one": 1}),
                }],
            },
            &mut output,
        )
        .unwrap();
    for old in ["missing", "\""] {
        assert!(
            suite
                .execute(
                    directory.path(),
                    case,
                    &Step::Replace {
                        path: path.clone(),
                        old: old.into(),
                        new: "two".into()
                    },
                    &mut output
                )
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(directory.path().join(&path)).unwrap(),
            "{\"one\":1}"
        );
    }
    suite
        .execute(
            directory.path(),
            case,
            &Step::Replace {
                path: path.clone(),
                old: "one".into(),
                new: "two".into(),
            },
            &mut output,
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join(path)).unwrap(),
        "{\"two\":1}"
    );
}

#[cfg(unix)]
#[test]
fn file_steps_reject_symlink_escapes() {
    let project = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), project.path().join("link")).unwrap();
    assert!(local_path(project.path(), Path::new("link/file")).is_err());
}

#[test]
fn authorization_and_environment_values_are_redacted_in_valid_json_reports() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let mut suite = ready(
        &fixture(),
        directory.path(),
        "http://127.0.0.1:1",
        "taku-live-report",
    )
    .unwrap();
    suite.redactor.add("token-\"quoted\\value");
    let path = directory.path().join("report.json");
    suite.save(&path, &json!({
        "output": "Bearer mock-secret-token / mock-secret-token / http://127.0.0.1:1 / token-\"quoted\\value",
        "spec": {"kind": "http", "path": "/widgets/mock-secret-token", "body": {"mock-secret-token": true},
            "assertions": [{"kind": "json", "pointer": "", "equals": {"mock-secret-token": "mock-secret-token"}}]},
    })).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("mock-secret-token")
            && !text.contains("127.0.0.1")
            && !text.contains("quoted")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["output"],
        "[REDACTED] / [REDACTED] / [REDACTED] / [REDACTED]"
    );
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["spec"]["path"], "/widgets/[REDACTED]");
    assert_eq!(saved["spec"]["body"], json!({"[REDACTED]": true}));
    assert_eq!(
        saved["spec"]["assertions"][0]["equals"],
        json!({"[REDACTED]": "[REDACTED]"})
    );
    assert!(
        suite
            .save(&path, &json!({"mock-secret-token": 1, "[REDACTED]": 2}))
            .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert!(suite.redactor.check("Bearer mock-secret-token").is_err());
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("response.json"),
        "{\"leaked\":\"mock-secret-token\"}",
    )
    .unwrap();
    assert!(scrub_project(&project, &suite.redactor).unwrap());
    assert_eq!(
        fs::read_to_string(project.join("response.json")).unwrap(),
        "{\"leaked\":\"[REDACTED]\"}"
    );
    assert!(!scrub_project(&project, &suite.redactor).unwrap());
}

#[test]
fn missing_opt_in_never_becomes_a_default_external_run() {
    for permission in [None, Some("0"), Some("true"), Some("yes")] {
        assert!(
            live_settings(|key| (key == "TAKU_LIVE_ALLOW_MUTATIONS")
                .then(|| permission.map(String::from))
                .flatten())
            .is_err()
        );
    }
    assert!(live_settings(|key| (key == "TAKU_LIVE_ALLOW_MUTATIONS").then(|| "1".into())).is_err());
}

#[test]
fn independent_http_does_not_follow_redirects() {
    let directory = TempDir::new().unwrap();
    catalogs(directory.path());
    let api = MockApi::start(Behavior::Normal);
    let suite = ready(&fixture(), directory.path(), &api.url, "taku-live-redirect").unwrap();
    let mut output = Observation::default();
    suite
        .execute(
            directory.path(),
            &suite.cases[0],
            &Step::Http {
                method: HttpMethod::Get,
                path: "/redirect".into(),
                body: None,
                status: 302,
                assertions: vec![],
            },
            &mut output,
        )
        .unwrap();
    assert_eq!(output.http_status, Some(302));
    assert_eq!(
        api.requests.lock().unwrap().as_slice(),
        &[("GET".into(), "/redirect".into())]
    );
}

#[test]
fn inherited_git_location_cannot_redirect_project_initialization() {
    let project = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let mut output = Observation::default();
    capture(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(project.path())
            .env("GIT_DIR", outside.path().join("redirected.git"))
            .env("GIT_WORK_TREE", outside.path())
            .timeout(TIMEOUT),
        &mut output,
    )
    .unwrap();
    assert!(project.path().join(".git").is_dir());
    assert!(!outside.path().join("redirected.git").exists());
}

#[cfg(unix)]
#[test]
fn subprocess_timeout_retains_output_and_does_not_hang() {
    let mut output = Observation::default();
    let start = std::time::Instant::now();
    let result = capture(
        Command::new("sh")
            .args(["-c", "printf timeout-marker; exec sleep 10"])
            .timeout(Duration::from_millis(50)),
        &mut output,
    );
    assert!(result.is_err());
    assert_eq!(output.stdout, "timeout-marker");
    assert!(start.elapsed() < Duration::from_secs(5));
}
