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
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, captured_content_types, ending) =
            (bodies.clone(), content_types.clone(), stop.clone());
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
                let response=match(request.method(),request.url()){(&Method::Post,"/api/saved_objects/_export"|"/s/esdiag/api/saved_objects/_export")=>Response::from_string(if body.contains("\"objects\""){"{\"id\":\"obj-1\",\"type\":\"visualization\",\"attributes\":{\"title\":\"Chart\",\"visState\":\"{\\\"a\\\":1}\",\"yaml\":\"# keep\\nx: 1\\n\"}}\n{\"exportedCount\":1,\"missingRefCount\":0}\n"}else{"{\"id\":\"9.4.2\",\"type\":\"config\",\"attributes\":{}}\n{\"id\":\"9.4.2\",\"type\":\"config-global\",\"attributes\":{}}\n{\"id\":\"obj-1\",\"type\":\"visualization\",\"sort\":[1],\"attributes\":{\"title\":\"Chart\",\"visState\":\"{\\\"a\\\":1}\",\"yaml\":\"# keep\\nx: 1\\n\"}}\n{\"exportedCount\":3,\"missingRefCount\":0}\n"}).with_header(Header::from_bytes("content-type","application/x-ndjson").unwrap()),(&Method::Post,path)if path.starts_with("/api/saved_objects/_import")||path.starts_with("/s/esdiag/api/saved_objects/_import")=>Response::from_string("{\"success\":true}"),_=>Response::from_string("not found").with_status_code(404)};
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            bodies,
            content_types,
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
    let listed = run(
        &project,
        &[
            "list",
            "--remote",
            "--type",
            "saved_objects",
            "--namespace",
            "esdiag",
            "--id",
            "obj-1",
        ],
    );
    assert_eq!(listed["result"][0]["id"], "obj-1");
    run(
        &project,
        &[
            "add",
            "--type",
            "saved_objects",
            "--namespace",
            "esdiag",
            "--id",
            "obj-1",
        ],
    );
    let path = project
        .path()
        .join("kb/esdiag/saved_objects/Chart-e7a05abc.json");
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
            "--type",
            "saved_objects",
            "--namespace",
            "esdiag",
            "--id",
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
        .join(".taku/cache/dev/kb/esdiag/saved_objects.yml");
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
            "--type",
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
    assert!(import.contains("filename=\"export.ndjson\""));
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
        &["app", "add", "kibana", "kb", "--url", &fake.url],
    );

    let definition_path = project
        .path()
        .join(".taku/applications/kibana/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["spaces"]["operations"]
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

    run(
        &project,
        &["fetch", "--type", "spaces", "--id", "new-space"],
    );
    let pushed = run(
        &project,
        &[
            "push",
            "--type",
            "spaces",
            "--id",
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
        &["app", "add", "kibana", "kb", "--url", "http://invalid"],
    );

    let definition_path = project
        .path()
        .join(".taku/applications/kibana/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["saved_objects"]["namespaced"] =
        serde_yaml::Value::Bool(true);
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
        .args(["--namespace", "../outside", "list"])
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
        &["app", "add", "kibana", "kb", "--url", &fake.url],
    );

    let definition_path = project
        .path()
        .join(".taku/applications/kibana/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    let saved_objects = &mut definition["target_profile"]["resource_types"]["saved_objects"];
    saved_objects["namespaced"] = serde_yaml::Value::Bool(true);
    saved_objects["operations"]["list"]["path"] =
        serde_yaml::Value::String("/s/{namespace}/api/saved_objects/_export".into());
    saved_objects["operations"]["list"]["default_namespace_path"] =
        serde_yaml::Value::String("/api/saved_objects/_export".into());
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();

    let named = run(
        &project,
        &[
            "list",
            "--remote",
            "--type",
            "saved_objects",
            "--namespace",
            "esdiag",
            "--id",
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
            "--type",
            "saved_objects",
            "--namespace",
            "default",
            "--id",
            "obj-1",
        ],
    );
    assert_eq!(default["result"][0]["namespace"], "default");
    assert_eq!(default["result"][0]["id"], "obj-1");

    let bodies = fake.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
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
        &["app", "add", "kibana", "kb", "--url", &fake.url],
    );

    let definition_path = project
        .path()
        .join(".taku/applications/kibana/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["workflows"] = serde_yaml::to_value(json!({
        "id": {"pointer": "/id", "scope": "universal"},
        "display_name": {"pointer": "/name", "strategy": "name"},
        "namespaced": true,
        "write_intent": "upsert",
        "transformations": [{"kind": "remove", "pointer": "/createdAt"}],
        "operations": {
            "read": {
                "method": "GET",
                "path": "/s/{namespace}/api/workflows/workflow/{id}",
                "default_namespace_path": "/api/workflows/workflow/{id}",
                "cardinality": "one"
            },
            "list": {
                "method": "GET",
                "path": "/s/{namespace}/api/workflows",
                "default_namespace_path": "/api/workflows",
                "cardinality": "many",
                "extract": "/results",
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
                "path": "/s/{namespace}/api/workflows/workflow",
                "default_namespace_path": "/api/workflows/workflow",
                "cardinality": "one"
            },
            "update": {
                "method": "PUT",
                "path": "/s/{namespace}/api/workflows/workflow/{id}",
                "default_namespace_path": "/api/workflows/workflow/{id}",
                "cardinality": "one",
                "transformations": [{"kind": "omit", "pointer": "/id"}]
            }
        }
    }))
    .unwrap();
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();

    let listed = run(
        &project,
        &[
            "list",
            "--remote",
            "--namespace",
            "default",
            "--type",
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
        &["fetch", "--namespace", "default", "--type", "workflows"],
    );
    run(
        &project,
        &[
            "push",
            "--namespace",
            "default",
            "--type",
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
