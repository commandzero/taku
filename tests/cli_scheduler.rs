use assert_cmd::Command;
use serde_json::{Value, json};
use std::process::Command as StdCommand;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Method, Response, Server};

struct Metrics {
    active: AtomicUsize,
    max_active: AtomicUsize,
    heavy: AtomicUsize,
    max_heavy: AtomicUsize,
    counts: Mutex<std::collections::BTreeMap<String, usize>>,
    fail_once: Mutex<Option<String>>,
    malformed_once: Mutex<Option<String>>,
    reads_absent: AtomicBool,
}
struct Fake {
    url: String,
    metrics: Arc<Metrics>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fake {
    fn start(fail_once: Option<&str>) -> Self {
        Self::start_with(fail_once, None)
    }
    fn start_malformed(path: &str) -> Self {
        Self::start_with(None, Some(path))
    }
    fn start_with(fail_once: Option<&str>, malformed_once: Option<&str>) -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let metrics = Arc::new(Metrics {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            heavy: AtomicUsize::new(0),
            max_heavy: AtomicUsize::new(0),
            counts: Mutex::new(Default::default()),
            fail_once: Mutex::new(fail_once.map(str::to_owned)),
            malformed_once: Mutex::new(malformed_once.map(str::to_owned)),
            reads_absent: AtomicBool::new(false),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (m, ending) = (metrics.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(request) = server.recv_timeout(Duration::from_millis(25)).unwrap() else {
                    continue;
                };
                let m = m.clone();
                thread::spawn(move || handle(request, m));
            }
        });
        Self {
            url,
            metrics,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn handle(mut request: tiny_http::Request, metrics: Arc<Metrics>) {
    let path = request.url().to_owned();
    *metrics
        .counts
        .lock()
        .unwrap()
        .entry(format!("{} {path}", request.method()))
        .or_default() += 1;
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body).unwrap();
    if path == "/" {
        request
            .respond(Response::from_string(r#"{"version":{"number":"9.4.0"}}"#))
            .unwrap();
        return;
    }
    if request.method() == &Method::Put {
        let active = metrics.active.fetch_add(1, Ordering::SeqCst) + 1;
        metrics.max_active.fetch_max(active, Ordering::SeqCst);
        let is_heavy = path.starts_with("/heavy/");
        if is_heavy {
            let heavy = metrics.heavy.fetch_add(1, Ordering::SeqCst) + 1;
            metrics.max_heavy.fetch_max(heavy, Ordering::SeqCst);
        }
        thread::sleep(Duration::from_millis(150));
        let fail = metrics
            .fail_once
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|selected| selected == &path);
        if fail {
            *metrics.fail_once.lock().unwrap() = None;
        }
        let malformed = metrics
            .malformed_once
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|selected| selected == &path);
        if malformed {
            *metrics.malformed_once.lock().unwrap() = None;
        }
        if is_heavy {
            metrics.heavy.fetch_sub(1, Ordering::SeqCst);
        }
        metrics.active.fetch_sub(1, Ordering::SeqCst);
        let response = if fail {
            Response::from_string("temporary").with_status_code(503)
        } else if malformed {
            Response::from_string("{")
                .with_header(Header::from_bytes("content-type", "application/json").unwrap())
        } else {
            Response::from_string(body)
                .with_header(Header::from_bytes("content-type", "application/json").unwrap())
        };
        request.respond(response).unwrap();
    } else if metrics.reads_absent.load(Ordering::SeqCst) {
        request
            .respond(Response::from_string("not found").with_status_code(404))
            .unwrap();
    } else {
        let parts: Vec<_> = path.trim_matches('/').split('/').collect();
        let value = json!({"id":parts[1],"name":parts[1],"value":0});
        request
            .respond(
                Response::from_string(value.to_string())
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap()),
            )
            .unwrap();
    }
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
fn output(project: &TempDir, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["--output", "json"])
        .args(args)
        .output()
        .unwrap()
}
fn setup(fake: &Fake, include_heavy: bool, include_dependent: bool) -> TempDir {
    let project = tempfile::tempdir().unwrap();
    assert!(
        StdCommand::new("git")
            .args(["init", "-q"])
            .current_dir(project.path())
            .status()
            .unwrap()
            .success()
    );
    run(
        &project,
        &["init", "--layout", "single", "--environment", "dev"],
    );
    run(&project, &["install", "elasticsearch"]);
    run(
        &project,
        &["target", "add", "elasticsearch", "es", "--url", &fake.url],
    );
    let mut resource_types = String::from(
        r#"  light:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/light/{id}" }
        upsert: { method: PUT, path: "/light/{id}", response: resource, retry_safe: false }
"#,
    );
    if include_heavy {
        resource_types.push_str(r#"  heavy:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/heavy/{id}" }
        upsert: { method: PUT, path: "/heavy/{id}", concurrency: serial, response: resource, retry_safe: false }
"#);
    }
    if include_dependent {
        resource_types.push_str(
            r#"  dependent:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      dependencies: [heavy]
      operations:
        read: { method: GET, path: "/dependent/{id}" }
        upsert: { method: PUT, path: "/dependent/{id}", response: resource, retry_safe: false }
        delete: { method: DELETE, path: "/dependent/{id}" }
"#,
        );
    }
    let definition = format!(
        "schema_version: 1\nversion: scheduler-definition\napplication: {{ name: elasticsearch, version: \">=9.0.0, <10.0.0\" }}\nresource_types:\n{resource_types}"
    );
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/version-9.yml"),
        definition,
    )
    .unwrap();
    let project_path = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_path).unwrap()).unwrap();
    config["max_requests"] = serde_yaml::Value::Number(3.into());
    std::fs::write(project_path, serde_yaml::to_string(&config).unwrap()).unwrap();
    for (kind, ids) in if include_dependent {
        vec![
            ("light", vec!["l1", "l2"]),
            ("heavy", vec!["h1", "h2"]),
            ("dependent", vec!["d1"]),
        ]
    } else if include_heavy {
        vec![("light", vec!["l1", "l2"]), ("heavy", vec!["h1", "h2"])]
    } else {
        vec![("light", vec!["l1", "l2"])]
    } {
        let dir = project.path().join(format!("es/{kind}"));
        std::fs::create_dir_all(&dir).unwrap();
        for id in ids {
            std::fs::write(
                dir.join(format!("{id}.json")),
                serde_json::to_string_pretty(&json!({"id":id,"name":id,"value":0})).unwrap(),
            )
            .unwrap();
        }
    }
    run(&project, &["fetch"]);
    for entry in walk_resources(&project) {
        let mut value: Value =
            serde_json::from_str(&std::fs::read_to_string(&entry).unwrap()).unwrap();
        value["value"] = json!(1);
        std::fs::write(entry, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }
    project
}
fn walk_resources(project: &TempDir) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for kind in ["light", "heavy", "dependent"] {
        if let Ok(entries) = std::fs::read_dir(project.path().join(format!("es/{kind}"))) {
            out.extend(entries.filter_map(Result::ok).map(|e| e.path()));
        }
    }
    out
}

#[test]
fn scheduler_respects_global_capacity_and_serial_non_overlap() {
    let fake = Fake::start(None);
    let project = setup(&fake, true, false);
    let result = run(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );
    assert_eq!(result["result"].as_array().unwrap().len(), 4);
    assert!(fake.metrics.max_active.load(Ordering::SeqCst) >= 2);
    assert!(fake.metrics.max_active.load(Ordering::SeqCst) <= 3);
    assert_eq!(fake.metrics.max_heavy.load(Ordering::SeqCst), 1);
}

#[test]
fn identical_retry_uses_journal_and_does_not_repeat_confirmed_success() {
    let fake = Fake::start(Some("/light/l2"));
    let project = setup(&fake, false, false);
    let first = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );
    assert_eq!(first.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(
        report["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["outcome"] == "failed")
    );
    run(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );
    let counts = fake.metrics.counts.lock().unwrap();
    assert_eq!(counts["PUT /light/l1"], 1);
    assert_eq!(counts["PUT /light/l2"], 2);
    let journal =
        std::fs::read_to_string(project.path().join(".taku/journals/dev/push.yml")).unwrap();
    assert!(journal.contains("l1:write"));
    assert!(journal.contains("l2:write"));
}

#[test]
fn failed_resource_type_blocks_dependents_while_independent_work_continues() {
    let fake = Fake::start(Some("/heavy/h1"));
    let project = setup(&fake, true, true);
    let output = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );
    assert_eq!(output.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "dependent" && item["outcome"] == "blocked_dependency")
    );
    let counts = fake.metrics.counts.lock().unwrap();
    assert!(!counts.contains_key("PUT /dependent/d1"));
    assert!(counts.contains_key("PUT /light/l1"));
}

#[test]
fn failed_resource_type_blocks_dependent_deletion_markers() {
    let fake = Fake::start(Some("/heavy/h1"));
    let project = setup(&fake, true, true);
    run(&project, &["remove", "es", "dependent", "d1"]);

    let output = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    assert_eq!(output.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "dependent" && item["outcome"] == "blocked_dependency")
    );
    assert!(
        !fake
            .metrics
            .counts
            .lock()
            .unwrap()
            .contains_key("DELETE /dependent/d1")
    );
}

#[test]
fn target_baselines_reject_unknown_schema_versions() {
    let fake = Fake::start(None);
    let project = setup(&fake, false, false);
    let path = project.path().join(".taku/baselines/dev/es.yml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replacen("schema_version: 1", "schema_version: 99", 1),
    )
    .unwrap();

    let failed = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("schema version"));
}

#[test]
fn push_journals_reject_unknown_schema_versions() {
    let fake = Fake::start(None);
    let project = setup(&fake, false, false);
    let path = project.path().join(".taku/journals/dev/push.yml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "schema_version: 99\nbinding: stale\ntotal: 1\ncompleted: {}\n",
    )
    .unwrap();

    let failed = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("schema version"));
}

#[test]
fn response_transformation_conflicts_do_not_abort_independent_resources() {
    let fake = Fake::start_malformed("/light/l2");
    let project = setup(&fake, false, false);

    let output = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    assert_eq!(output.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "l1" && item["outcome"] == "success")
    );
    assert!(
        report["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "l2" && item["outcome"] == "transformation_conflict")
    );
    let counts = fake.metrics.counts.lock().unwrap();
    assert_eq!(counts["PUT /light/l1"], 1);
    assert_eq!(counts["PUT /light/l2"], 1);
}

#[test]
fn outbound_transformation_conflicts_are_reported_before_any_network_call() {
    let fake = Fake::start(None);
    let project = setup(&fake, false, false);
    let definition_path = project
        .path()
        .join(".taku/applications/elasticsearch/version-9.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["resource_types"]["light"][0]["operations"]["upsert"]["body"] =
        serde_yaml::Value::String("/missing".into());
    std::fs::write(
        &definition_path,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    fake.metrics.counts.lock().unwrap().clear();

    let output = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    assert_eq!(
        output.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["result"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["outcome"] == "transformation_conflict"),
        "{report}"
    );
    assert!(fake.metrics.counts.lock().unwrap().is_empty());
}

#[test]
fn retry_safe_by_default_create_with_client_owned_id_may_retry() {
    let fake = Fake::start(Some("/light/l1"));
    let project = setup(&fake, false, false);
    let definition_path = project
        .path()
        .join(".taku/applications/elasticsearch/version-9.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    let light = &mut definition["resource_types"]["light"][0];
    light["write_intent"] = serde_yaml::Value::String("create".into());
    light["operations"]["create"] =
        serde_yaml::from_str("{ method: PUT, path: \"/light/{id}\" }\n").unwrap();
    std::fs::write(
        &definition_path,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    std::fs::remove_file(project.path().join(".taku/cache/dev/es/light.yml")).unwrap();
    std::fs::remove_file(project.path().join(".taku/baselines/dev/es.yml")).unwrap();
    fake.metrics.reads_absent.store(true, Ordering::SeqCst);
    run(&project, &["fetch"]);
    fake.metrics.counts.lock().unwrap().clear();

    run(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    let counts = fake.metrics.counts.lock().unwrap();
    assert_eq!(counts["PUT /light/l1"], 2);
    assert_eq!(counts["PUT /light/l2"], 1);
}
