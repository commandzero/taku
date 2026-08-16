use assert_cmd::Command;
use serde_json::{Value, json};
use std::process::Command as StdCommand;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Method, Response, Server};

#[derive(Clone, Debug)]
struct Captured {
    method: String,
    path: String,
    authorization: Option<String>,
    if_match: Option<String>,
    body: String,
}

struct FakeTarget {
    url: String,
    version: Arc<Mutex<String>>,
    requests: Arc<Mutex<Vec<Captured>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeTarget {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let resource = Arc::new(Mutex::new(
            json!({"id":"pipe-1","name":"Pipeline","description":"remote","processors":[],"version":"7","created_date_millis":1000,"modified_date_millis":2000,"_secret":"NEVER-PERSIST"}),
        ));
        let version = Arc::new(Mutex::new(String::from("1")));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (state, remote_version, captured, stopping) = (
            resource.clone(),
            version.clone(),
            requests.clone(),
            stop.clone(),
        );
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let authorization = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("authorization"))
                    .map(|h| h.value.as_str().to_owned());
                let if_match = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("if-match"))
                    .map(|header| header.value.as_str().to_owned());
                captured.lock().unwrap().push(Captured {
                    method: request.method().as_str().into(),
                    path: request.url().into(),
                    authorization,
                    if_match,
                    body: body.clone(),
                });
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/_ingest/pipeline/pipe-1") => Response::from_string(
                        json!({"pipe-1": state.lock().unwrap().clone()}).to_string(),
                    )
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap()),
                    (&Method::Get, "/_ingest/pipeline/pipe-2") => Response::from_string(
                        json!({
                            "pipe-2": {
                                "name": "Second",
                                "description": "adopt me",
                                "processors": []
                            }
                        })
                        .to_string(),
                    )
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap()),
                    (&Method::Get, "/version") => Response::from_string(
                        json!({"version":remote_version.lock().unwrap().clone()}).to_string(),
                    )
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap()),
                    (&Method::Get, "/_ingest/pipeline") => Response::from_string(
                        json!({
                            "pipe-1": state.lock().unwrap().clone(),
                            "pipe-2": {"name":"Second","description":"adopt me","processors":[]}
                        })
                        .to_string(),
                    )
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap()),
                    (&Method::Put, path) if path.starts_with("/_ingest/pipeline/pipe-1") => {
                        *state.lock().unwrap() = serde_json::from_str(&body).unwrap();
                        Response::from_string("{\"acknowledged\":true}").with_header(
                            Header::from_bytes("content-type", "application/json").unwrap(),
                        )
                    }
                    (&Method::Delete, "/_ingest/pipeline/pipe-1") => {
                        Response::from_string("{\"acknowledged\":true}")
                    }
                    (&Method::Post, "/jobs") => Response::from_string(
                        "{\"id\":\"job-1\",\"name\":\"Pending Job\",\"enabled\":true}",
                    )
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap()),
                    _ => Response::from_string("not found").with_status_code(404),
                };
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            version,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

#[test]
fn changed_target_facts_that_select_a_new_variant_block_push_until_pull_reconciles() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let definition = r#"schema_version: 1
application: { name: elasticsearch, version: "variant-test" }
target_profile:
  headers: { content-type: application/json }
  fact_probes:
    - name: version
      pointer: /version
      operation: { method: GET, path: /version, cardinality: one }
  resource_types:
    ingest_pipelines:
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/_ingest/pipeline/{id}", cardinality: one, extract: "/{id}" }
        upsert: { method: PUT, path: "/_ingest/pipeline/{id}", cardinality: one }
      variants:
        - { name: v1, facts: { version: "1" } }
        - { name: v2, facts: { version: "2" } }
"#;
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/resources.yml"),
        definition,
    )
    .unwrap();
    run(&project, &["fetch"]);
    assert!(
        std::fs::read_to_string(project.path().join(".taku/baselines/dev/es.yml"))
            .unwrap()
            .contains("v1")
    );
    *target.version.lock().unwrap() = "2".into();
    let desired = project.path().join("es/ingest_pipelines/Pipeline.json");
    let mut local: Value =
        serde_json::from_str(&std::fs::read_to_string(&desired).unwrap()).unwrap();
    local["description"] = json!("local-v2");
    std::fs::write(&desired, serde_json::to_string_pretty(&local).unwrap()).unwrap();
    let blocked = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("different Resource Type Variants"));
    run(&project, &["fetch"]);
    run(&project, &["pull", "--yes"]);
    assert!(
        std::fs::read_to_string(project.path().join(".taku/baselines/dev/es.yml"))
            .unwrap()
            .contains("v2")
    );
}

#[test]
fn target_scoped_pending_create_becomes_identified_only_after_trustworthy_success() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let definition = r#"schema_version: 1
application: { name: elasticsearch, version: "test" }
target_profile:
  headers: { content-type: application/json }
  fact_probes: []
  resource_types:
    jobs:
      id: { pointer: /id, scope: target }
      display_name: { pointer: /name, strategy: name }
      write_intent: create
      operations:
        read: { method: GET, path: "/jobs/{id}", cardinality: one }
        create: { method: POST, path: "/jobs", cardinality: one, trustworthy_response: true }
"#;
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/resources.yml"),
        definition,
    )
    .unwrap();
    let dir = project.path().join("es/jobs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Pending Job.json");
    std::fs::write(&path, "{ name: 'Pending Job', enabled: true }").unwrap();
    run(&project, &["fetch", "--type", "jobs"]);
    let result = run(
        &project,
        &[
            "push",
            "--type",
            "jobs",
            "--untracked",
            "allow",
            "--uncommitted",
            "allow",
        ],
    );
    assert_eq!(result["result"][0]["outcome"], "success");
    let created: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(created["id"], "job-1");
    assert_eq!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/jobs")
            .count(),
        1
    );
}

#[test]
fn provider_precedence_and_dotenv_validation_do_not_expose_credentials() {
    let target = FakeTarget::start();
    let project = setup(&target);
    std::fs::write(
        project.path().join("credentials.env"),
        b"\xef\xbb\xbfTOKEN=\"Bearer FILE-SENTINEL\"\n",
    )
    .unwrap();
    let process = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .env("TOKEN", "Bearer PROCESS-SENTINEL")
        .args(["--output", "json", "fetch"])
        .output()
        .unwrap();
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    assert!(!String::from_utf8_lossy(&process.stdout).contains("PROCESS-SENTINEL"));
    assert_eq!(
        target
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .authorization
            .as_deref(),
        Some("Bearer PROCESS-SENTINEL")
    );
    let cli = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args([
            "--output",
            "json",
            "--set",
            "authorization=Bearer CLI-SENTINEL",
            "fetch",
        ])
        .output()
        .unwrap();
    assert!(cli.status.success());
    assert_eq!(
        target
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .authorization
            .as_deref(),
        Some("Bearer CLI-SENTINEL")
    );
    std::fs::write(
        project.path().join("credentials.env"),
        "TOKEN=FIRST-SECRET\nTOKEN=SECOND-SECRET\n",
    )
    .unwrap();
    let duplicate = output(&project, &["fetch"]);
    assert!(!duplicate.status.success());
    let error = String::from_utf8_lossy(&duplicate.stderr);
    assert!(error.contains("duplicate key TOKEN"));
    assert!(!error.contains("FIRST-SECRET"));
    assert!(!error.contains("SECOND-SECRET"));
}

#[test]
fn remote_list_add_remove_and_forget_preserve_partial_inventory_safety() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);

    let remote = run(
        &project,
        &[
            "list",
            "--remote",
            "--untracked",
            "--type",
            "ingest_pipelines",
        ],
    );
    assert_eq!(remote["result"].as_array().unwrap().len(), 1);
    assert_eq!(remote["result"][0]["id"], "pipe-2");
    run(
        &project,
        &["add", "--type", "ingest_pipelines", "--id", "pipe-2"],
    );
    assert!(
        project
            .path()
            .join("es/ingest_pipelines/Second-pipe-2.json")
            .is_file()
    );

    let request_count = target.requests.lock().unwrap().len();
    run(&project, &["remove", "--id", "pipe-1"]);
    assert_eq!(target.requests.lock().unwrap().len(), request_count);
    let markers: Vec<_> = std::fs::read_dir(project.path().join("es/ingest_pipelines"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".delete.yml"))
        .collect();
    assert_eq!(markers.len(), 1);
    assert!(
        !project
            .path()
            .join("es/ingest_pipelines/Pipeline.json")
            .exists()
    );
    run(&project, &["forget", "--id", "pipe-1"]);
    assert!(!markers[0].path().exists());
    assert_eq!(target.requests.lock().unwrap().len(), request_count);
}

#[test]
fn push_deletes_only_through_a_matching_guarded_marker_and_consumes_it() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    run(&project, &["remove", "--id", "pipe-1"]);
    let result = run(
        &project,
        &["push", "--uncommitted", "allow", "--untracked", "allow"],
    );
    assert_eq!(result["result"][0]["outcome"], "deleted");
    assert!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "DELETE")
    );
    assert!(
        !std::fs::read_dir(project.path().join("es/ingest_pipelines"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".delete.yml"))
    );
}

#[test]
fn fetch_explicitly_reobserves_resources_marked_for_deletion() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    run(&project, &["remove", "--id", "pipe-1"]);
    target.requests.lock().unwrap().clear();

    let fetched = run(&project, &["fetch"]);

    assert_eq!(fetched["result"][0]["id"], "pipe-1");
    assert!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "GET" && request.path == "/_ingest/pipeline/pipe-1")
    );
}

#[test]
fn deletion_markers_reject_unknown_schema_versions() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    run(&project, &["remove", "--id", "pipe-1"]);
    let marker = std::fs::read_dir(project.path().join("es/ingest_pipelines"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().ends_with(".delete.yml"))
        .unwrap()
        .path();
    let text = std::fs::read_to_string(&marker).unwrap();
    std::fs::write(
        &marker,
        text.replacen("schema_version: 1", "schema_version: 99", 1),
    )
    .unwrap();

    let failed = output(&project, &["forget", "--id", "pipe-1"]);

    assert!(!failed.status.success());
    assert!(marker.exists());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("schema version"));
}

#[test]
fn deletion_marker_binding_must_match_the_tree_that_contains_it() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(
        &project,
        &["app", "add", "elasticsearch", "other", "--url", &target.url],
    );
    run(&project, &["fetch"]);
    run(&project, &["remove", "--id", "pipe-1"]);
    let marker = std::fs::read_dir(project.path().join("es/ingest_pipelines"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().ends_with(".delete.yml"))
        .unwrap()
        .path();
    let text = std::fs::read_to_string(&marker).unwrap();
    std::fs::write(&marker, text.replacen("target: es", "target: other", 1)).unwrap();
    let requests = target.requests.lock().unwrap().len();

    let failed = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    assert!(!failed.status.success());
    assert_eq!(target.requests.lock().unwrap().len(), requests);
    assert!(String::from_utf8_lossy(&failed.stderr).contains("binding"));
}

#[cfg(unix)]
#[test]
fn deletion_marker_discovery_rejects_symlinked_inputs() {
    use std::os::unix::fs::symlink;
    let target = FakeTarget::start();
    let project = setup(&target);
    let outside = project.path().join("outside.delete.yml");
    std::fs::write(
        &outside,
        "schema_version: 1\nenvironment: dev\ntarget: es\ntype: ingest_pipelines\nid: pipe-1\nguard: guard\nsource_path: ignored\n",
    )
    .unwrap();
    let linked = project.path().join("es/ingest_pipelines/Linked.delete.yml");
    symlink(&outside, &linked).unwrap();

    let failed = output(&project, &["forget", "--id", "pipe-1"]);

    assert!(!failed.status.success());
    assert!(linked.exists());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("symlinked"));
}

#[test]
fn guarded_concurrency_uses_a_response_token_without_persisting_the_response_only_field() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let definition = r#"schema_version: 1
application: { name: elasticsearch, version: guard-test }
target_profile:
  headers: { content-type: application/json }
  fact_probes: []
  resource_types:
    ingest_pipelines:
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      concurrency_mode: guarded
      guard_pointer: /version
      transformations: [{ kind: remove, pointer: /version }]
      operations:
        read: { method: GET, path: "/_ingest/pipeline/{id}", cardinality: one, extract: "/{id}" }
        upsert: { method: PUT, path: "/_ingest/pipeline/{id}", cardinality: one, guard_header: if-match }
"#;
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/resources.yml"),
        definition,
    )
    .unwrap();
    run(&project, &["fetch"]);
    let cache = std::fs::read_to_string(
        project
            .path()
            .join(".taku/cache/dev/es/ingest_pipelines.yml"),
    )
    .unwrap();
    assert!(!cache.lines().any(|line| matches!(
        line.trim(),
        "version: '7'" | "version: \"7\"" | "version: 7"
    )));
    assert!(
        cache.contains("guard: '7'")
            || cache.contains("guard: \"7\"")
            || cache.contains("guard: 7")
    );
    let path = project.path().join("es/ingest_pipelines/Pipeline.json");
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["description"] = json!("guarded");
    std::fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    target.requests.lock().unwrap().clear();
    let planned = run(
        &project,
        &[
            "push",
            "--dry-run",
            "--untracked",
            "allow",
            "--uncommitted",
            "allow",
        ],
    );
    assert_eq!(planned["result"][0]["outcome"], "planned");
    assert_eq!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == "GET" && request.path == "/_ingest/pipeline/pipe-1")
            .count(),
        1
    );
    assert!(
        !target
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "PUT")
    );
    target.requests.lock().unwrap().clear();
    run(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );
    assert_eq!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .find(|request| request.method == "PUT")
            .unwrap()
            .if_match
            .as_deref(),
        Some("7")
    );
}

#[test]
fn create_only_dry_run_rechecks_remote_absence_instead_of_trusting_the_cache() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let definition_path = project
        .path()
        .join(".taku/applications/elasticsearch/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["ingest_pipelines"]["write_intent"] =
        serde_yaml::Value::String("create".into());
    std::fs::write(
        &definition_path,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    run(&project, &["fetch"]);
    let cache_path = project
        .path()
        .join(".taku/cache/dev/es/ingest_pipelines.yml");
    let mut cache: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cache_path).unwrap()).unwrap();
    cache["resources"]["pipe-1"]["present"] = serde_yaml::Value::Bool(false);
    cache["resources"]["pipe-1"]["value"] = serde_yaml::Value::Null;
    std::fs::write(&cache_path, serde_yaml::to_string(&cache).unwrap()).unwrap();
    target.requests.lock().unwrap().clear();

    let failed = output(
        &project,
        &[
            "push",
            "--dry-run",
            "--untracked",
            "allow",
            "--uncommitted",
            "allow",
        ],
    );

    assert!(!failed.status.success());
    let report: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(report["result"][0]["outcome"], "creation_conflict");
    assert_eq!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == "GET" && request.path == "/_ingest/pipeline/pipe-1")
            .count(),
        1
    );
}

#[test]
fn fetch_reports_inbound_transformation_failures_as_structured_conflicts() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let definition_path = project
        .path()
        .join(".taku/applications/elasticsearch/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["ingest_pipelines"]["transformations"] =
        serde_yaml::from_str("- { kind: extract, pointer: /missing }\n").unwrap();
    std::fs::write(
        &definition_path,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();

    let fetched = output(&project, &["fetch"]);

    assert_eq!(fetched.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&fetched.stdout).unwrap();
    assert_eq!(report["result"][0]["outcome"], "transformation_conflict");
}

#[test]
fn patch_mutation_mode_compares_and_pulls_only_fields_owned_by_the_resource() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let definition_path = project
        .path()
        .join(".taku/applications/elasticsearch/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["ingest_pipelines"]["mutation_mode"] =
        serde_yaml::Value::String("patch".into());
    std::fs::write(
        &definition_path,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    let desired_path = project.path().join("es/ingest_pipelines/Pipeline.json");
    std::fs::write(
        &desired_path,
        serde_json::to_string_pretty(&json!({"id":"pipe-1","name":"Pipeline","processors":[]}))
            .unwrap(),
    )
    .unwrap();

    run(&project, &["fetch"]);
    let status = run(&project, &["status"]);
    assert_eq!(status["result"][0]["state"], "in_sync");
    let mut local: Value =
        serde_json::from_str(&std::fs::read_to_string(&desired_path).unwrap()).unwrap();
    local["enabled"] = json!(true);
    std::fs::write(&desired_path, serde_json::to_string_pretty(&local).unwrap()).unwrap();
    let pulled = run(&project, &["pull", "--yes"]);
    assert_eq!(pulled["result"][0]["outcome"], "local_only");

    let desired: Value =
        serde_json::from_str(&std::fs::read_to_string(desired_path).unwrap()).unwrap();
    assert!(desired.get("description").is_none());
    assert!(desired.get("version").is_none());
    assert_eq!(desired["enabled"], true);
}

#[test]
fn diff_rejects_observed_state_bound_to_different_project_inputs() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    let project_file = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_file).unwrap()).unwrap();
    config["environments"]["dev"]["targets"]["es"]["headers"] =
        serde_yaml::from_str("x-changed: yes\n").unwrap();
    std::fs::write(&project_file, serde_yaml::to_string(&config).unwrap()).unwrap();

    let failed = output(&project, &["diff"]);

    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("structurally invalid"));
}

#[test]
fn pull_conflicts_prevent_all_resource_writes_in_the_invocation() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    let first = project.path().join("es/ingest_pipelines/Pipeline.json");
    let first_before = std::fs::read_to_string(&first).unwrap();
    let second = project.path().join("es/ingest_pipelines/Second.json");
    let original = json!({"id":"pipe-2","name":"Second","description":"original","processors":[]});
    std::fs::write(&second, serde_json::to_string_pretty(&original).unwrap()).unwrap();
    let cache_path = project
        .path()
        .join(".taku/cache/dev/es/ingest_pipelines.yml");
    let mut observation: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cache_path).unwrap()).unwrap();
    observation["resources"]["pipe-2"] = serde_yaml::to_value(json!({
        "local_hash": "baseline-that-matches-neither-side",
        "path": "es/ingest_pipelines/Second.json",
        "present": true,
        "value": {"id":"pipe-2","name":"Second","description":"remote","processors":[]},
        "guard": "guard-2"
    }))
    .unwrap();
    std::fs::write(&cache_path, serde_yaml::to_string(&observation).unwrap()).unwrap();
    let divergent =
        json!({"id":"pipe-2","name":"Second","description":"local-change","processors":[]});
    std::fs::write(&second, serde_json::to_string_pretty(&divergent).unwrap()).unwrap();

    let pulled = output(&project, &["pull", "--yes"]);

    assert_eq!(pulled.status.code(), Some(4));
    assert_eq!(std::fs::read_to_string(&first).unwrap(), first_before);
    assert!(
        std::fs::read_to_string(&second)
            .unwrap()
            .contains("local-change")
    );
}

impl Drop for FakeTarget {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn setup(target: &FakeTarget) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        StdCommand::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    run(
        &dir,
        &["init", "--layout", "single", "--environment", "dev"],
    );
    run(&dir, &["install", "elasticsearch"]);
    run(
        &dir,
        &["app", "add", "elasticsearch", "es", "--url", &target.url],
    );
    let path = dir.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("Pipeline.json"),
        serde_json::to_string_pretty(
            &json!({"id":"pipe-1","name":"Pipeline","description":"local","processors":[]}),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("credentials.env"),
        "TOKEN=\"Bearer SENTINEL-CREDENTIAL\"\n",
    )
    .unwrap();
    let project_path = dir.path().join(".taku/project.yml");
    let mut project: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_path).unwrap()).unwrap();
    project["environments"]["dev"]["targets"]["es"]["auth"] =
        serde_yaml::from_str("dotenv: credentials.env\nfields:\n  authorization: TOKEN\n").unwrap();
    std::fs::write(project_path, serde_yaml::to_string(&project).unwrap()).unwrap();
    dir
}

fn output(project: &TempDir, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .arg("--output")
        .arg("json")
        .args(args)
        .output()
        .unwrap()
}

fn run(project: &TempDir, args: &[&str]) -> Value {
    let output = output(project, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn fetch_status_pull_and_push_use_generic_operations_without_leaking_secrets() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let desired = project.path().join("es/ingest_pipelines/Pipeline.json");
    let before = std::fs::read_to_string(&desired).unwrap();

    let fetched = run(&project, &["fetch"]);
    assert_eq!(fetched["result"][0]["outcome"], "observed");
    assert_eq!(std::fs::read_to_string(&desired).unwrap(), before);
    let cache = std::fs::read_to_string(
        project
            .path()
            .join(".taku/cache/dev/es/ingest_pipelines.yml"),
    )
    .unwrap();
    assert!(!cache.contains("NEVER-PERSIST"));
    assert!(!cache.contains("created_date_millis"));
    assert!(!cache.contains("modified_date_millis"));
    assert!(!cache.contains("SENTINEL-CREDENTIAL"));
    assert!(
        !String::from_utf8_lossy(&output(&project, &["fetch"]).stdout)
            .contains("SENTINEL-CREDENTIAL")
    );
    assert_eq!(
        target.requests.lock().unwrap()[0].authorization.as_deref(),
        Some("Bearer SENTINEL-CREDENTIAL")
    );

    let status = run(&project, &["status"]);
    assert_eq!(status["result"][0]["state"], "drift");
    assert_eq!(
        output(&project, &["status", "--check"]).status.code(),
        Some(3)
    );
    let diff = run(&project, &["diff"]);
    assert_eq!(diff["result"][0]["id"], "pipe-1");

    run(&project, &["pull", "--yes"]);
    assert!(
        std::fs::read_to_string(&desired)
            .unwrap()
            .contains("remote")
    );
    let mut changed: Value =
        serde_json::from_str(&std::fs::read_to_string(&desired).unwrap()).unwrap();
    changed["description"] = json!("published");
    std::fs::write(&desired, serde_json::to_string_pretty(&changed).unwrap()).unwrap();
    run(
        &project,
        &["push", "--uncommitted", "allow", "--untracked", "allow"],
    );
    let requests = target.requests.lock().unwrap();
    let put = requests.iter().find(|r| r.method == "PUT").unwrap();
    assert!(put.path.starts_with("/_ingest/pipeline/pipe-1"));
    assert!(put.body.contains("published"));
    assert!(!put.body.contains("\"id\""));
    assert!(!put.body.contains("created_date_millis"));
    assert!(!put.body.contains("modified_date_millis"));
    assert!(!put.body.contains("NEVER-PERSIST"));
}
