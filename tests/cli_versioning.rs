use assert_cmd::Command;
use resource_control::{CompletionIntent, CompletionQuery, completion_candidates};
use serde_json::Value;
use std::process::Command as StdCommand;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Method, Response, Server};

struct Fake {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Fake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, ending) = (requests.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(request) = server.recv_timeout(Duration::from_millis(25)).unwrap() else {
                    continue;
                };
                captured
                    .lock()
                    .unwrap()
                    .push(format!("{} {}", request.method(), request.url()));
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/missing-version") => {
                        Response::from_string("not found").with_status_code(404)
                    }
                    (&Method::Get, "/version") => {
                        Response::from_string(r#"{"product":{"version":"9.4.0"}}"#)
                    }
                    (&Method::Get, "/widgets") => Response::from_string(
                        r#"[{"id":"one","name":"One"}]"#,
                    )
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap()),
                    _ => Response::from_string("unexpected").with_status_code(404),
                };
                request.respond(response).unwrap();
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

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn version_endpoints_fall_back_in_order_and_unavailable_resource_types_are_skipped() {
    let fake = Fake::start();
    let project = project(&fake);
    let mut query = CompletionQuery::new(project.path(), CompletionIntent::RemoteResourceType);
    query.target = Some("api".into());
    assert!(
        completion_candidates(&query)
            .unwrap()
            .iter()
            .any(|candidate| candidate.value == "widgets")
    );
    assert!(!project.path().join(".taku/baselines/dev/api.yml").exists());
    let result = run(&project, &["list", "--remote", "api", "widgets"]);

    assert_eq!(result["result"][0]["id"], "one");
    assert_eq!(
        *fake.requests.lock().unwrap(),
        vec![
            "GET /missing-version",
            "GET /version",
            "GET /missing-version",
            "GET /version",
            "GET /widgets"
        ]
    );
    let baseline =
        std::fs::read_to_string(project.path().join(".taku/baselines/dev/api.yml")).unwrap();
    assert!(baseline.contains("application_version: 9.4.0"));
    assert!(baseline.contains("widgets"));
    assert!(!baseline.contains("future_widgets"));

    let unavailable = project.path().join("api/future_widgets");
    std::fs::create_dir_all(&unavailable).unwrap();
    std::fs::write(
        unavailable.join("Future.json"),
        r#"{"id":"future","name":"Future"}"#,
    )
    .unwrap();
    std::fs::write(
        unavailable.join(".resource.yaml"),
        "schema_version: 1\nmetadata: { track: true }\n",
    )
    .unwrap();
    let failed = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["list", "api", "future_widgets"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("is unavailable"));

    std::fs::remove_file(unavailable.join("Future.json")).unwrap();
    let request_count = fake.requests.lock().unwrap().len();
    let failed = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["list", "--remote", "api", "widgets"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("is unavailable"));
    assert_eq!(fake.requests.lock().unwrap().len(), request_count);
}

#[test]
fn a_fresh_target_rejects_version_specific_resource_hints_before_discovery() {
    let fake = Fake::start();
    let project = project(&fake);
    let unavailable = project.path().join("api/future_widgets");
    std::fs::create_dir_all(&unavailable).unwrap();
    std::fs::write(
        unavailable.join(".resource.yaml"),
        "schema_version: 1\nmetadata: { track: true }\n",
    )
    .unwrap();

    let failed = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["list", "--remote", "api", "widgets"])
        .output()
        .unwrap();

    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("is unavailable"));
    assert!(fake.requests.lock().unwrap().is_empty());
}

fn project(fake: &Fake) -> TempDir {
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
        &["target", "add", "elasticsearch", "api", "--url", &fake.url],
    );
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/application.yml"),
        r#"schema_version: 1
version: test
application: { name: elasticsearch }
target_profile: { headers: {} }
version_endpoints:
  - { method: GET, path: /missing-version, pointer: /version }
  - { method: GET, path: /version, pointer: /product/version }
"#,
    )
    .unwrap();
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/version-9.yml"),
        r#"schema_version: 1
version: test
application: { name: elasticsearch, version: ">=9.0.0, <10.0.0" }
resource_types:
  widgets:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/widgets/{id}", cardinality: one }
        list: { method: GET, path: /widgets, cardinality: many }
        upsert: { method: PUT, path: "/widgets/{id}", cardinality: one }
  future_widgets:
    - version: ">=9.5.0, <10.0.0"
      stability: preview
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/future/{id}", cardinality: one }
        list: { method: GET, path: /future, cardinality: many }
        upsert: { method: PUT, path: "/future/{id}", cardinality: one }
"#,
    )
    .unwrap();
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/version-8.yml"),
        r#"schema_version: 1
version: test
application: { name: elasticsearch, version: ">=8.0.0, <9.0.0" }
resource_types:
  widgets:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/widgets/{id}", cardinality: one }
        list: { method: GET, path: /widgets, cardinality: many }
        upsert: { method: PUT, path: "/widgets/{id}", cardinality: one }
"#,
    )
    .unwrap();
    project
}

fn run(project: &TempDir, args: &[&str]) -> Value {
    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["--output", "json"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
