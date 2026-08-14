use assert_cmd::Command;
use serde_json::Value;
use std::process::Command as StdCommand;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

struct Fake {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Fake {
    fn start(duplicate: bool) -> Self {
        Self::start_with(duplicate, false)
    }
    fn start_malformed() -> Self {
        Self::start_with(false, true)
    }
    fn start_with(duplicate: bool, malformed: bool) -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let ending = stop.clone();
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(request) = server.recv_timeout(Duration::from_millis(25)).unwrap() else {
                    continue;
                };
                let body = if malformed {
                    "{".to_owned()
                } else if request.url() == "/widgets" {
                    r#"{"items":[{"id":"a","name":"A"}],"next":"second"}"#.to_owned()
                } else {
                    let id = if duplicate { "a" } else { "b" };
                    format!(r#"{{"items":[{{"id":"{id}","name":"B"}}]}}"#)
                };
                request
                    .respond(Response::from_string(body).with_header(
                        Header::from_bytes("content-type", "application/json").unwrap(),
                    ))
                    .unwrap();
            }
        });
        Self {
            url,
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

fn setup(fake: &Fake) -> TempDir {
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
        &["app", "add", "elasticsearch", "api", "--url", &fake.url],
    );
    std::fs::write(
        project
            .path()
            .join(".taku/applications/elasticsearch/resources.yml"),
        r#"schema_version: 1
application: { name: elasticsearch, version: pagination-test }
target_profile:
  headers: {}
  fact_probes: []
  resource_types:
    widgets:
      id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/widgets/{id}", cardinality: one }
        list:
          method: GET
          path: /widgets
          cardinality: many
          extract: /items
          pagination:
            kind: cursor
            cursor_parameter: cursor
            next_pointer: /next
            max_pages: 3
        upsert: { method: PUT, path: "/widgets/{id}", cardinality: one }
"#,
    )
    .unwrap();
    project
}

fn output(project: &TempDir) -> std::process::Output {
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["--output", "json", "list", "--remote", "--type", "widgets"])
        .output()
        .unwrap()
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

#[test]
fn cursor_pagination_preserves_envelope_continuation_across_item_extraction() {
    let fake = Fake::start(false);
    let project = setup(&fake);

    let listed: Value = serde_json::from_slice(&output(&project).stdout).unwrap();

    assert_eq!(listed["result"].as_array().unwrap().len(), 2);
    assert_eq!(listed["result"][0]["id"], "a");
    assert_eq!(listed["result"][1]["id"], "b");
}

#[test]
fn pagination_rejects_duplicate_ids_across_pages() {
    let fake = Fake::start(true);
    let project = setup(&fake);

    let listed = output(&project);

    assert!(!listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stderr).contains("duplicate Resource ID"));
}

#[test]
fn list_reports_inbound_transformation_failures_in_its_output_envelope() {
    let fake = Fake::start_malformed();
    let project = setup(&fake);

    let listed = output(&project);

    assert_eq!(listed.status.code(), Some(4));
    let report: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(report["result"][0]["outcome"], "transformation_conflict");
}
