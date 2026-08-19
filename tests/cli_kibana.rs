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
    content_types: Arc<Mutex<Vec<String>>>,
    urls: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

struct UpsertFake {
    url: String,
    methods: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

struct OperationTransformFake {
    url: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

struct SkillFake {
    url: String,
    content: Arc<Mutex<String>>,
    bodies: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

struct PluginFake {
    url: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl PluginFake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let installed = Arc::new(AtomicBool::new(false));
        let (captured, ending, state) = (requests.clone(), stop.clone(), installed.clone());
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push((format!("{} {}", request.method(), request.url()), body));
                let plugin = json!({
                    "created_at": "2025-01-01T00:00:00.000Z",
                    "description": "Financial analysis tools and skills for Claude",
                    "id": "financial-analysis",
                    "manifest": {
                        "author": {"name": "Anthropic", "url": "https://www.anthropic.com"},
                        "keywords": ["finance", "analysis"],
                        "repository": "https://github.com/anthropics/financial-services-plugins"
                    },
                    "name": "financial-analysis",
                    "skill_ids": ["financial-analysis-analyze-portfolio"],
                    "source_url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis",
                    "unmanaged_assets": {
                        "agents": [],
                        "hooks": [],
                        "lsp_servers": [],
                        "mcp_servers": [],
                        "output_styles": []
                    },
                    "updated_at": "2025-01-01T00:00:00.000Z",
                    "version": "1.0.0"
                });
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/api/status") => {
                        Response::from_string(r#"{"version":{"number":"9.4.0"}}"#)
                    }
                    (&Method::Get, "/api/agent_builder/plugins/financial-analysis")
                        if state.load(Ordering::Relaxed) =>
                    {
                        Response::from_string(plugin.to_string())
                    }
                    (&Method::Get, "/api/agent_builder/plugins/financial-analysis") => {
                        Response::from_string("not found").with_status_code(404)
                    }
                    (&Method::Get, "/api/agent_builder/plugins") => {
                        let results = if state.load(Ordering::Relaxed) {
                            vec![plugin]
                        } else {
                            Vec::new()
                        };
                        Response::from_string(json!({"results": results}).to_string())
                    }
                    (&Method::Post, "/api/agent_builder/plugins/install") => {
                        state.store(true, Ordering::Relaxed);
                        Response::from_string(plugin.to_string())
                    }
                    (&Method::Delete, "/api/agent_builder/plugins/financial-analysis") => {
                        state.store(false, Ordering::Relaxed);
                        Response::from_string(r#"{"success":true}"#)
                    }
                    _ => Response::from_string("unexpected request").with_status_code(405),
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

impl Drop for PluginFake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

impl SkillFake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let content = Arc::new(Mutex::new(
            "# Agentic Diagnostic Assistant\n\nUse the referenced runbook.\n".to_owned(),
        ));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let ending = stop.clone();
        let served_content = content.clone();
        let captured_bodies = bodies.clone();
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                if !body.is_empty() {
                    captured_bodies.lock().unwrap().push(body);
                }
                let skill = json!({
                    "id": "agentic-diagnostic-assistant",
                    "name": "Agentic Diagnostic Assistant",
                    "description": "Diagnose Elastic Stack cases",
                    "experimental": true,
                    "metadata": {"owner": "support"},
                    "content": served_content.lock().unwrap().clone(),
                    "referenced_content": [{
                        "name": "runbook",
                        "relativePath": "./references",
                        "content": "# Runbook\n\nInspect diagnostics.\n"
                    }]
                });
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/api/status") => {
                        Response::from_string(r#"{"version":{"number":"9.4.0"}}"#)
                    }
                    (&Method::Get, "/api/agent_builder/skills") => {
                        Response::from_string(json!({"results": [skill]}).to_string())
                    }
                    (&Method::Get, "/api/agent_builder/skills/agentic-diagnostic-assistant") => {
                        Response::from_string(skill.to_string())
                    }
                    (&Method::Put, "/api/agent_builder/skills/agentic-diagnostic-assistant") => {
                        Response::from_string(r#"{"ok":true}"#)
                    }
                    _ => Response::from_string("not found").with_status_code(404),
                };
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            content,
            bodies,
            stop,
            thread: Some(thread),
        }
    }

    fn set_content(&self, content: &str) {
        *self.content.lock().unwrap() = content.to_owned();
    }
}

impl Drop for SkillFake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

impl OperationTransformFake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, ending) = (requests.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push((format!("{} {}", request.method(), request.url()), body));
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/api/status") => {
                        Response::from_string(r#"{"version":{"number":"9.4.0"}}"#)
                    }
                    (&Method::Get, "/api/workflows/workflow/create-me") => {
                        Response::from_string("not found").with_status_code(404)
                    }
                    (&Method::Get, "/api/workflows/workflow/update-me") => Response::from_string(
                        r#"{"id":"update-me","name":"Old","enabled":false,"createdAt":"server-only"}"#,
                    ),
                    (&Method::Get, "/api/workflows?page=1&size=100") => Response::from_string(
                        r#"{"page":1,"size":100,"total":1,"results":[{"id":"listed","name":"Listed","enabled":true}]}"#,
                    ),
                    (&Method::Post, "/api/workflows/workflow")
                    | (&Method::Put, "/api/workflows/workflow/update-me") => {
                        Response::from_string(r#"{"ok":true}"#)
                    }
                    _ => Response::from_string("unexpected request").with_status_code(405),
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

impl Drop for OperationTransformFake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

impl UpsertFake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let methods = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, ending) = (methods.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(request) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                    continue;
                };
                captured
                    .lock()
                    .unwrap()
                    .push(format!("{} {}", request.method(), request.url()));
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/api/status") => {
                        Response::from_string(r#"{"version":{"number":"9.4.0"}}"#)
                    }
                    (&Method::Get, "/api/spaces/space/new-space") => {
                        Response::from_string("not found").with_status_code(404)
                    }
                    (&Method::Post, "/api/spaces/space") => {
                        Response::from_string(r#"{"id":"new-space","name":"New Space"}"#)
                    }
                    _ => Response::from_string("unexpected request").with_status_code(405),
                };
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            methods,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for UpsertFake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
impl Fake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let content_types = Arc::new(Mutex::new(Vec::new()));
        let urls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, captured_content_types, captured_urls, ending) = (
            bodies.clone(),
            content_types.clone(),
            urls.clone(),
            stop.clone(),
        );
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let content_type = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("content-type"))
                    .map(|header| header.value.as_str().to_owned())
                    .unwrap_or_default();
                captured.lock().unwrap().push(body.clone());
                captured_content_types.lock().unwrap().push(content_type);
                captured_urls.lock().unwrap().push(request.url().to_owned());
                let response=match(request.method(),request.url()){(&Method::Get,"/api/status")=>Response::from_string(r#"{"version":{"number":"9.4.0"}}"#),(&Method::Post,"/api/saved_objects/_export"|"/s/esdiag/api/saved_objects/_export")=>Response::from_string(if body.contains("5e05b9ee-3e49-4efd-8a16-94de208ebb83"){"{\"attributes\":{\"color\":\"#48EFCF\",\"description\":\"Elastic Stack Diagnostics (ESDiag)\",\"name\":\"ESDiag\"},\"id\":\"5e05b9ee-3e49-4efd-8a16-94de208ebb83\",\"references\":[],\"type\":\"tag\"}\n{\"exportedCount\":1,\"missingRefCount\":0}\n"}else if body.contains("\"objects\""){"{\"id\":\"obj-1\",\"type\":\"visualization\",\"attributes\":{\"title\":\"Chart\",\"visState\":\"{\\\"a\\\":1}\",\"yaml\":\"# keep\\nx: 1\\n\"}}\n{\"exportedCount\":1,\"missingRefCount\":0}\n"}else{"{\"id\":\"9.4.2\",\"type\":\"config\",\"attributes\":{}}\n{\"id\":\"9.4.2\",\"type\":\"config-global\",\"attributes\":{}}\n{\"id\":\"obj-1\",\"type\":\"visualization\",\"sort\":[1],\"attributes\":{\"title\":\"Chart\",\"visState\":\"{\\\"a\\\":1}\",\"yaml\":\"# keep\\nx: 1\\n\"}}\n{\"attributes\":{\"color\":\"#48EFCF\",\"description\":\"Elastic Stack Diagnostics (ESDiag)\",\"name\":\"ESDiag\"},\"id\":\"5e05b9ee-3e49-4efd-8a16-94de208ebb83\",\"references\":[],\"type\":\"tag\"}\n{\"exportedCount\":4,\"missingRefCount\":0}\n"}).with_header(Header::from_bytes("content-type","application/x-ndjson").unwrap()),(&Method::Post,path)if path.starts_with("/api/saved_objects/_import")||path.starts_with("/s/esdiag/api/saved_objects/_import")=>Response::from_string("{\"success\":true}"),_=>Response::from_string("not found").with_status_code(404)};
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            bodies,
            content_types,
            urls,
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

fn configure_skill_projection(project: &TempDir) {
    let definition_path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    let skill = &mut definition["resource_types"]["skills"][0];
    skill["display_name"] = serde_yaml::to_value(json!({
        "pointer": "/name",
        "strategy": "id",
        "unique": true
    }))
    .unwrap();
    skill["filesystem"] = serde_yaml::to_value(json!({
        "split": "frontmatter_markdown",
        "merge": "frontmatter_markdown",
        "frontmatter_markdown": {
            "document": "SKILL.md",
            "body_pointer": "/content",
            "referenced_files": {
                "pointer": "/referenced_content",
                "path_pointer": "/relativePath",
                "name_pointer": "/name",
                "content_pointer": "/content",
                "extension": "md"
            }
        }
    }))
    .unwrap();
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );
    let listed = run(
        &project,
        &[
            "list",
            "--remote",
            "kb",
            "saved_objects",
            "--namespace",
            "esdiag",
            "obj-1",
        ],
    );
    assert_eq!(listed["result"][0]["id"], "obj-1");
    run(
        &project,
        &[
            "add",
            "kb",
            "saved_objects",
            "--namespace",
            "esdiag",
            "obj-1",
        ],
    );
    let path = project
        .path()
        .join("kb/esdiag/saved_objects/Chart-obj-1.json");
    let canonical = std::fs::read_to_string(&path).unwrap();
    assert!(canonical.contains("\"visState\": {"));
    assert!(canonical.contains("# keep\\nx: 1"));
    assert!(!canonical.contains("\"sort\""));
    assert!(
        !project
            .path()
            .join("kb/esdiag/saved_objects/export.ndjson")
            .exists()
    );
    run(
        &project,
        &[
            "fetch",
            "kb",
            "saved_objects",
            "--namespace",
            "esdiag",
            "obj-1",
        ],
    );
    let mut value: Value = serde_json::from_str(&canonical).unwrap();
    value["attributes"]["visState"]["a"] = json!(2);
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    let second_path = project.path().join("kb/esdiag/saved_objects/Second.json");
    let second = json!({
        "id": "obj-2",
        "type": "visualization",
        "attributes": {"title":"Second","visState":{"a":3},"yaml":"x: 2\n"}
    });
    std::fs::write(&second_path, serde_json::to_string_pretty(&second).unwrap()).unwrap();
    let cache_path = project
        .path()
        .join(".taku/cache/dev/kb/esdiag/saved_objects.yaml");
    let mut cache: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&cache_path).unwrap()).unwrap();
    cache["resources"]["obj-2"] = serde_yaml::to_value(json!({
        "local_hash":"different",
        "path":"kb/esdiag/saved_objects/Second.json",
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
            "kb",
            "saved_objects",
            "--namespace",
            "esdiag",
            "--untracked",
            "allow",
            "--uncommitted",
            "allow",
        ],
    );
    let bodies = fake.bodies.lock().unwrap();
    let import_index = bodies
        .iter()
        .position(|body| body.contains("\\\"a\\\":2"))
        .expect("NDJSON import body");
    let import = &bodies[import_index];
    assert!(
        fake.content_types.lock().unwrap()[import_index]
            .starts_with("multipart/form-data; boundary=")
    );
    assert!(import.contains("name=\"file\""));
    assert!(import.contains("filename=\"saved_objects.ndjson\""));
    assert!(
        fake.urls
            .lock()
            .unwrap()
            .iter()
            .any(|url| url == "/s/esdiag/api/saved_objects/_import?overwrite=true")
    );
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
fn json_list_and_map_bundles_keep_their_declared_shape() {
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );
    let definition_path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["resource_types"]["map_items"] = serde_yaml::to_value(json!([{
        "id": {"pointer": "/id", "scope": "universal"},
        "display_name": {"pointer": "/name", "strategy": "name"},
        "write_intent": "upsert",
        "operations": {
            "read": {"method": "GET", "path": "/api/map_items/{id}", "cardinality": "one"},
            "upsert": {
                "method": "POST",
                "path": "/api/saved_objects/_import",
                "cardinality": "many",
                "bundle": {"shape": "map", "format": "json"}
            }
        }
    }]))
    .unwrap();
    definition["resource_types"]["list_items"] = serde_yaml::to_value(json!([{
        "id": {"pointer": "/id", "scope": "universal"},
        "display_name": {"pointer": "/name", "strategy": "name"},
        "write_intent": "upsert",
        "operations": {
            "read": {"method": "GET", "path": "/api/list_items/{id}", "cardinality": "one"},
            "upsert": {
                "method": "POST",
                "path": "/api/saved_objects/_import",
                "cardinality": "many",
                "bundle": {"shape": "list", "format": "json"}
            }
        }
    }]))
    .unwrap();
    std::fs::write(
        &definition_path,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    let directory = project.path().join("kb/map_items");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("Two.json"),
        serde_json::to_string_pretty(&json!({"id": "two", "name": "Two", "value": 2})).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.join("One.json"),
        serde_json::to_string_pretty(&json!({"id": "one", "name": "One", "value": 1})).unwrap(),
    )
    .unwrap();
    let list_directory = project.path().join("kb/list_items");
    std::fs::create_dir_all(&list_directory).unwrap();
    std::fs::write(
        list_directory.join("Only.json"),
        serde_json::to_string_pretty(&json!({"id": "only", "name": "Only", "value": 1})).unwrap(),
    )
    .unwrap();
    run(&project, &["fetch", "kb", "map_items"]);
    run(&project, &["fetch", "kb", "list_items"]);
    run(
        &project,
        &[
            "push",
            "kb",
            "map_items",
            "--missing",
            "restore",
            "--untracked",
            "allow",
            "--uncommitted",
            "allow",
        ],
    );
    run(
        &project,
        &[
            "push",
            "kb",
            "list_items",
            "--missing",
            "restore",
            "--untracked",
            "allow",
            "--uncommitted",
            "allow",
        ],
    );

    let bodies = fake.bodies.lock().unwrap();
    let body: Value = bodies
        .iter()
        .filter_map(|body| serde_json::from_str(body).ok())
        .find(|body: &Value| body.get("one").is_some())
        .unwrap();
    assert_eq!(
        body,
        json!({
            "one": {"name": "One", "value": 1},
            "two": {"name": "Two", "value": 2}
        })
    );
    let list: Value = bodies
        .iter()
        .filter_map(|body| serde_json::from_str(body).ok())
        .find(|body: &Value| body.is_array())
        .unwrap();
    assert_eq!(list, json!([{"id": "only", "name": "Only", "value": 1}]));
}

#[test]
fn saved_object_filenames_use_the_first_available_display_name_pointer() {
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );

    run(
        &project,
        &[
            "add",
            "kb",
            "saved_objects",
            "--namespace",
            "esdiag",
            "5e05b9ee-3e49-4efd-8a16-94de208ebb83",
        ],
    );

    assert!(
        project
            .path()
            .join("kb/esdiag/saved_objects/ESDiag-208ebb83.json")
            .is_file()
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
        &["target", "add", "kibana", "kb", "--url", "http://invalid"],
    );
    let dir = project.path().join("kb/spaces");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Space.json5"),"// deprecated API\n{ id: 'space-1', name: 'Space', description: \"\"\"line one\nline two\"\"\" }").unwrap();
    let listed = run(&project, &["list", "kb", "spaces"]);
    assert_eq!(listed["result"][0]["id"], "space-1");
}

#[test]
fn frontmatter_markdown_resources_merge_from_a_directory_tree() {
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
        &["target", "add", "kibana", "kb", "--url", "http://invalid"],
    );

    configure_skill_projection(&project);

    let skill = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    std::fs::create_dir_all(skill.join("references")).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nid: agentic-diagnostic-assistant\nname: Agentic Diagnostic Assistant\ndescription: Diagnose Elastic Stack cases\nexperimental: true\nmetadata:\n  owner: support\n---\n\n# Agentic Diagnostic Assistant\n\nUse the referenced runbook.\n",
    )
    .unwrap();
    std::fs::write(
        skill.join("references/runbook.md"),
        "# Runbook\n\nInspect diagnostics.\n",
    )
    .unwrap();

    let listed = run(
        &project,
        &["list", "kb", "skills", "--namespace", "default"],
    );
    let resource = &listed["result"][0];
    assert_eq!(resource["id"], "agentic-diagnostic-assistant");
    assert_eq!(resource["name"], "Agentic Diagnostic Assistant");
    assert_eq!(
        resource["path"],
        "kb/default/skills/agentic-diagnostic-assistant"
    );
}

#[test]
fn adding_a_frontmatter_markdown_resource_splits_it_to_a_directory_tree() {
    let fake = SkillFake::start();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );
    configure_skill_projection(&project);

    run(
        &project,
        &[
            "add",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );

    let directory = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    let markdown = std::fs::read_to_string(directory.join("SKILL.md")).unwrap();
    assert!(markdown.starts_with("---\n"));
    assert!(markdown.contains("id: agentic-diagnostic-assistant"));
    assert!(markdown.contains("owner: support"));
    assert!(markdown.ends_with("# Agentic Diagnostic Assistant\n\nUse the referenced runbook.\n"));
    assert_eq!(
        std::fs::read_to_string(directory.join("references/runbook.md")).unwrap(),
        "# Runbook\n\nInspect diagnostics.\n"
    );
    assert!(!directory.with_extension("json").exists());
}

#[test]
fn pulling_a_frontmatter_markdown_resource_replaces_its_directory_projection() {
    let fake = SkillFake::start();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );

    run(
        &project,
        &[
            "add",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );
    run(
        &project,
        &[
            "fetch",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );
    fake.set_content("# Agentic Diagnostic Assistant\n\nUpdated remotely.\n");
    run(
        &project,
        &[
            "fetch",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );
    let pulled = run(
        &project,
        &[
            "pull",
            "--yes",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );

    assert_eq!(pulled["result"][0]["outcome"], "pulled");
    let markdown = std::fs::read_to_string(
        project
            .path()
            .join("kb/default/skills/agentic-diagnostic-assistant/SKILL.md"),
    )
    .unwrap();
    assert!(markdown.ends_with("# Agentic Diagnostic Assistant\n\nUpdated remotely.\n"));
}

#[test]
fn removing_a_projected_resource_replaces_the_directory_with_a_deletion_marker() {
    let fake = SkillFake::start();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );
    let selector = [
        "kb",
        "skills",
        "--namespace",
        "default",
        "agentic-diagnostic-assistant",
    ];
    let mut add = vec!["add"];
    add.extend(selector);
    run(&project, &add);
    let mut fetch = vec!["fetch"];
    fetch.extend(selector);
    run(&project, &fetch);

    let mut remove = vec!["remove"];
    remove.extend(selector);
    run(&project, &remove);

    let directory = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    assert!(!directory.exists());
    assert!(
        project
            .path()
            .join("kb/default/skills/agentic-diagnostic-assistant.delete.yaml")
            .is_file()
    );
}

#[test]
fn pushing_a_projected_resource_merges_passthrough_frontmatter_and_files() {
    let fake = SkillFake::start();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );
    let selector = [
        "kb",
        "skills",
        "--namespace",
        "default",
        "agentic-diagnostic-assistant",
    ];
    let mut add = vec!["add"];
    add.extend(selector);
    run(&project, &add);
    let mut fetch = vec!["fetch"];
    fetch.extend(selector);
    run(&project, &fetch);

    let directory = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    let markdown = std::fs::read_to_string(directory.join("SKILL.md")).unwrap();
    std::fs::write(
        directory.join("SKILL.md"),
        markdown.replace("owner: support", "owner: field-engineering"),
    )
    .unwrap();
    std::fs::write(
        directory.join("references/runbook.md"),
        "# Runbook\n\nInspect the updated diagnostics.\n",
    )
    .unwrap();
    let mut push = vec!["push", "--untracked", "allow", "--uncommitted", "allow"];
    push.extend(selector);
    run(&project, &push);

    let bodies = fake.bodies.lock().unwrap();
    let body: Value = serde_json::from_str(bodies.last().unwrap()).unwrap();
    assert_eq!(body["metadata"]["owner"], "field-engineering");
    assert_eq!(body["experimental"], true);
    assert_eq!(
        body["referenced_content"][0]["content"],
        "# Runbook\n\nInspect the updated diagnostics.\n"
    );
    assert!(
        body["content"]
            .as_str()
            .unwrap()
            .contains("Use the referenced runbook")
    );
}

#[test]
fn upsert_uses_create_when_a_resource_is_absent_and_no_native_upsert_exists() {
    let fake = UpsertFake::start();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );

    let definition_path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["resource_types"]["spaces"][0]["operations"]
        .as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String("upsert".into()));
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();

    let resources = project.path().join("kb/spaces");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("New Space.json"),
        r#"{"id":"new-space","name":"New Space"}"#,
    )
    .unwrap();

    run(&project, &["fetch", "kb", "spaces", "new-space"]);
    let pushed = run(
        &project,
        &[
            "push",
            "kb",
            "spaces",
            "new-space",
            "--missing",
            "restore",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );

    assert_eq!(pushed["result"][0]["outcome"], "success");
    let methods = fake.methods.lock().unwrap();
    assert!(methods.contains(&"POST /api/spaces/space".to_string()));
    assert!(!methods.iter().any(|request| request.starts_with("PUT ")));
}

#[test]
fn namespaced_resource_types_require_an_explicit_namespace_directory() {
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
        &["target", "add", "kibana", "kb", "--url", "http://invalid"],
    );

    let definition_path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["resource_types"]["saved_objects"][0]["namespaced"] = serde_yaml::Value::Bool(true);
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();

    for (namespace, id, title) in [
        ("esdiag", "dashboard-esdiag", "ESDiag Dashboard"),
        ("default", "dashboard-default", "Default Dashboard"),
    ] {
        let directory = project
            .path()
            .join("kb")
            .join(namespace)
            .join("saved_objects");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{title}.json")),
            serde_json::to_string_pretty(&json!({
                "id": id,
                "type": "dashboard",
                "attributes": {"title": title}
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let obsolete = project.path().join("kb/saved_objects");
    std::fs::create_dir_all(&obsolete).unwrap();
    std::fs::write(
        obsolete.join("Unscoped.json"),
        r#"{"id":"unscoped","type":"dashboard","attributes":{"title":"Unscoped"}}"#,
    )
    .unwrap();
    let spaces = project.path().join("kb/spaces");
    std::fs::create_dir_all(&spaces).unwrap();
    std::fs::write(
        spaces.join("ESDiag.json"),
        r#"{"id":"esdiag","name":"ESDiag"}"#,
    )
    .unwrap();

    let listed = run(&project, &["list"]);
    let resources = listed["result"].as_array().unwrap();
    assert_eq!(resources.len(), 3);
    assert!(resources.iter().any(|resource| {
        resource["type"] == "saved_objects"
            && resource["namespace"] == "esdiag"
            && resource["id"] == "dashboard-esdiag"
    }));
    assert!(resources.iter().any(|resource| {
        resource["type"] == "saved_objects"
            && resource["namespace"] == "default"
            && resource["id"] == "dashboard-default"
    }));
    assert!(resources.iter().any(|resource| {
        resource["type"] == "spaces"
            && resource.get("namespace").is_none()
            && resource["id"] == "esdiag"
    }));
    assert!(
        !resources
            .iter()
            .any(|resource| resource["id"] == "unscoped")
    );

    let traversal = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["list", "kb", "saved_objects", "--namespace", "../outside"])
        .output()
        .unwrap();
    assert!(!traversal.status.success());
    assert!(
        String::from_utf8_lossy(&traversal.stderr)
            .contains("Namespace must be one non-empty path segment")
    );
}

#[test]
fn namespace_selectors_render_named_and_default_operation_paths() {
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );

    let named = run(
        &project,
        &[
            "list",
            "--remote",
            "kb",
            "saved_objects",
            "--namespace",
            "esdiag",
            "obj-1",
        ],
    );
    assert_eq!(named["result"][0]["namespace"], "esdiag");
    assert_eq!(named["result"][0]["id"], "obj-1");

    let default = run(
        &project,
        &[
            "list",
            "--remote",
            "kb",
            "saved_objects",
            "--namespace",
            "default",
            "obj-1",
        ],
    );
    assert_eq!(default["result"][0]["namespace"], "default");
    assert_eq!(default["result"][0]["id"], "obj-1");

    let bodies = fake.bodies.lock().unwrap();
    assert_eq!(bodies.iter().filter(|body| !body.is_empty()).count(), 2);
    let urls = fake.urls.lock().unwrap();
    assert!(
        urls.iter()
            .any(|url| url == "/s/esdiag/api/saved_objects/_export")
    );
    assert!(urls.iter().any(|url| url == "/api/saved_objects/_export"));
}

#[test]
fn selected_write_operation_applies_its_own_outbound_transformations() {
    let fake = OperationTransformFake::start();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );

    let definition_path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["resource_types"]["workflows"] = serde_yaml::to_value(json!([{
        "version": ">=9.4.0, <10.0.0",
        "id": {"pointer": "/id", "scope": "universal"},
        "display_name": {"pointer": "/name", "strategy": "name"},
        "namespaced": true,
        "write_intent": "upsert",
        "transformations": [{"kind": "remove", "pointer": "/createdAt"}],
        "operations": {
            "read": {
                "method": "GET",
                "path": "/api/workflows/workflow/{id}",
                "namespace": {"prefix": "/s/{namespace}"},
                "cardinality": "one"
            },
            "list": {
                "method": "GET",
                "path": "/api/workflows",
                "namespace": {"prefix": "/s/{namespace}"},
                "cardinality": "many",
                "extract": "/results",
                "response": {"collection": "list"},
                "pagination": {
                    "kind": "page_size",
                    "page_parameter": "page",
                    "size_parameter": "size",
                    "size": 100,
                    "max_pages": 2
                }
            },
            "create": {
                "method": "POST",
                "path": "/api/workflows/workflow",
                "namespace": {"prefix": "/s/{namespace}"},
                "cardinality": "one"
            },
            "update": {
                "method": "PUT",
                "path": "/api/workflows/workflow/{id}",
                "namespace": {"prefix": "/s/{namespace}"},
                "cardinality": "one",
                "transformations": [{"kind": "omit", "pointer": "/id"}]
            }
        }
    }]))
    .unwrap();
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();

    let listed = run(
        &project,
        &[
            "list",
            "--remote",
            "--namespace",
            "default",
            "kb",
            "workflows",
        ],
    );
    assert_eq!(listed["result"][0]["id"], "listed");

    let directory = project.path().join("kb/default/workflows");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("Create.json"),
        r#"{"id":"create-me","name":"Create","enabled":true}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("Update.json"),
        r#"{"id":"update-me","name":"Update","enabled":true}"#,
    )
    .unwrap();

    run(
        &project,
        &["fetch", "--namespace", "default", "kb", "workflows"],
    );
    run(
        &project,
        &[
            "push",
            "--namespace",
            "default",
            "kb",
            "workflows",
            "--missing",
            "restore",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );

    let requests = fake.requests.lock().unwrap();
    let create = requests
        .iter()
        .find(|(request, _)| request == "POST /api/workflows/workflow")
        .map(|(_, body)| serde_json::from_str::<Value>(body).unwrap())
        .unwrap();
    let update = requests
        .iter()
        .find(|(request, _)| request == "PUT /api/workflows/workflow/update-me")
        .map(|(_, body)| serde_json::from_str::<Value>(body).unwrap())
        .unwrap();
    assert_eq!(create["id"], "create-me");
    assert!(update.get("id").is_none());
    assert_eq!(update["name"], "Update");
}

#[test]
fn agent_builder_plugins_install_from_source_and_delete_without_force() {
    let fake = PluginFake::start();
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
        &["target", "add", "kibana", "kb", "--url", &fake.url],
    );

    let directory = project.path().join("kb/default/plugins");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("financial-analysis.json"),
        serde_json::to_string_pretty(&json!({
            "id": "financial-analysis",
            "name": "financial-analysis",
            "source_url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis"
        }))
        .unwrap(),
    )
    .unwrap();

    let selector = [
        "kb",
        "plugins",
        "--namespace",
        "default",
        "financial-analysis",
    ];
    let mut fetch = vec!["fetch"];
    fetch.extend(selector);
    run(&project, &fetch);

    let mut push = vec!["push"];
    push.extend(selector);
    push.extend(["--uncommitted", "allow", "--untracked", "allow"]);
    let installed = run(&project, &push);
    assert_eq!(installed["result"][0]["outcome"], "success");

    run(&project, &fetch);
    let mut status = vec!["status"];
    status.extend(selector);
    let status = run(&project, &status);
    assert_eq!(status["result"][0]["state"], "in_sync");

    let listed = run(
        &project,
        &[
            "list",
            "--remote",
            "kb",
            "plugins",
            "--namespace",
            "default",
        ],
    );
    assert_eq!(listed["result"][0]["id"], "financial-analysis");
    assert_eq!(listed["result"][0]["name"], "financial-analysis");

    let mut forget = vec!["forget"];
    forget.extend(selector);
    run(&project, &forget);
    let mut add = vec!["add"];
    add.extend(selector);
    run(&project, &add);
    let canonical: Value = serde_json::from_str(
        &std::fs::read_to_string(directory.join("financial-analysis.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        canonical,
        json!({
            "id": "financial-analysis",
            "name": "financial-analysis",
            "source_url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis"
        })
    );

    let mut remove = vec!["remove"];
    remove.extend(selector);
    run(&project, &remove);
    let deleted = run(&project, &push);
    assert_eq!(deleted["result"][0]["outcome"], "deleted");

    let requests = fake.requests.lock().unwrap();
    let install_body = requests
        .iter()
        .find_map(|(request, body)| {
            (request == "POST /api/agent_builder/plugins/install").then_some(body)
        })
        .expect("plugin install request");
    assert_eq!(
        serde_json::from_str::<Value>(install_body).unwrap(),
        json!({
            "plugin_name": "financial-analysis",
            "url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis"
        })
    );
    assert!(
        requests.iter().any(|(request, _)| {
            request == "DELETE /api/agent_builder/plugins/financial-analysis"
        })
    );
    assert!(
        !requests
            .iter()
            .any(|(request, _)| request.contains("force=true"))
    );
}
