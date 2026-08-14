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

struct Fake {
    url: String,
    bodies: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, ending) = (bodies.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                captured.lock().unwrap().push(body);
                let response=match(request.method(),request.url()){(&Method::Post,"/api/saved_objects/_export")=>Response::from_string("{\"id\":\"obj-1\",\"type\":\"visualization\",\"attributes\":{\"title\":\"Chart\",\"visState\":\"{\\\"a\\\":1}\",\"yaml\":\"# keep\\nx: 1\\n\"}}\n{\"exportedCount\":1,\"missingRefCount\":0}\n").with_header(Header::from_bytes("content-type","application/x-ndjson").unwrap()),(&Method::Post,path)if path.starts_with("/api/saved_objects/_import")=>Response::from_string("{\"success\":true}"),_=>Response::from_string("not found").with_status_code(404)};
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            bodies,
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
fn kibana_ndjson_is_unbundled_to_canonical_resources_and_rebuilt_only_for_push() {
    let fake = Fake::start();
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
    run(&project, &["install", "kibana"]);
    run(
        &project,
        &["app", "add", "kibana", "kb", "--url", &fake.url],
    );
    let listed = run(&project, &["list", "--remote", "--type", "saved_objects"]);
    assert_eq!(listed["result"][0]["id"], "obj-1");
    run(
        &project,
        &["add", "--type", "saved_objects", "--id", "obj-1"],
    );
    let path = project.path().join("kb/saved_objects/Chart-e7a05abc.json");
    let canonical = std::fs::read_to_string(&path).unwrap();
    assert!(canonical.contains("\"visState\": {"));
    assert!(canonical.contains("# keep\\nx: 1"));
    assert!(
        !project
            .path()
            .join("kb/saved_objects/export.ndjson")
            .exists()
    );
    run(
        &project,
        &["fetch", "--type", "saved_objects", "--id", "obj-1"],
    );
    let mut value: Value = serde_json::from_str(&canonical).unwrap();
    value["attributes"]["visState"]["a"] = json!(2);
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    let second_path = project.path().join("kb/saved_objects/Second.json");
    let second = json!({
        "id": "obj-2",
        "type": "visualization",
        "attributes": {"title":"Second","visState":{"a":3},"yaml":"x: 2\n"}
    });
    std::fs::write(&second_path, serde_json::to_string_pretty(&second).unwrap()).unwrap();
    let cache_path = project.path().join(".taku/cache/dev/kb/saved_objects.yml");
    let mut cache: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cache_path).unwrap()).unwrap();
    cache["resources"]["obj-2"] = serde_yaml::to_value(json!({
        "local_hash":"different",
        "path":"kb/saved_objects/Second.json",
        "present":true,
        "value":{
            "id":"obj-2",
            "type":"visualization",
            "attributes":{"title":"Second","visState":{"a":1},"yaml":"x: 2\n"}
        },
        "guard":"obj-2-guard"
    }))
    .unwrap();
    std::fs::write(&cache_path, serde_yaml::to_string(&cache).unwrap()).unwrap();
    run(
        &project,
        &[
            "push",
            "--type",
            "saved_objects",
            "--untracked",
            "allow",
            "--uncommitted",
            "allow",
        ],
    );
    let bodies = fake.bodies.lock().unwrap();
    let import = bodies
        .iter()
        .find(|body| body.contains("\\\"a\\\":2"))
        .expect("NDJSON import body");
    assert!(import.ends_with('\n'));
    assert_eq!(import.lines().count(), 2);
    assert!(import.contains("\\\"a\\\":3"));
    assert_eq!(
        bodies
            .iter()
            .filter(|body| body.contains("\\\"a\\\":2") || body.contains("\\\"a\\\":3"))
            .count(),
        1
    );
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("\"objects\"") && body.contains("obj-1"))
    );
}

#[test]
fn local_json5_accepts_comments_and_triple_quoted_human_text() {
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
    run(&project, &["install", "kibana"]);
    run(
        &project,
        &["app", "add", "kibana", "kb", "--url", "http://invalid"],
    );
    let dir = project.path().join("kb/spaces");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Space.json5"),"// deprecated API\n{ id: 'space-1', name: 'Space', description: \"\"\"line one\nline two\"\"\" }").unwrap();
    let listed = run(&project, &["list", "--type", "spaces"]);
    assert_eq!(listed["result"][0]["id"], "space-1");
}
