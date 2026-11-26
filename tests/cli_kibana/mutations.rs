use super::{edit_catalog, run, setup};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Method, Response, Server};

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
#[test]
fn upsert_uses_create_when_a_resource_is_absent_and_no_native_upsert_exists() {
    let fake = UpsertFake::start();
    let project = setup(&fake.url);

    edit_catalog(&project, |definition| {
        definition["resource_types"]["spaces"][0]["operations"]
            .as_mapping_mut()
            .unwrap()
            .remove(serde_yaml::Value::String("upsert".into()));
    });

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
fn selected_write_operation_applies_its_own_outbound_transformations() {
    let fake = OperationTransformFake::start();
    let project = setup(&fake.url);

    configure_workflow_operations(&project);

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

fn configure_workflow_operations(project: &TempDir) {
    edit_catalog(project, |definition| {
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
    });
}
