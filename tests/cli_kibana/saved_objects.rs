use super::{edit_catalog, run, setup};
use assert_cmd::Command;
use serde_json::{Value, json};
use std::path::Path;
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
#[test]
fn kibana_ndjson_is_unbundled_to_canonical_resources_and_rebuilt_only_for_push() {
    let fake = Fake::start();
    let project = setup(&fake.url);
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
        .join("kb/esdiag/saved_objects/visualization/Chart-obj-1.json");
    let canonical = read_canonical_export(&project, &path);
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
    edit_saved_objects(&project, &path, &canonical);
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
    assert_ndjson_import(&fake);
}

fn assert_ndjson_import(fake: &Fake) {
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

fn edit_saved_objects(project: &TempDir, path: &Path, canonical: &str) {
    let mut value: Value = serde_json::from_str(canonical).unwrap();
    value["attributes"]["visState"]["a"] = json!(2);
    std::fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    let second_path = project
        .path()
        .join("kb/esdiag/saved_objects/visualization/Second.json");
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
        "path":"kb/esdiag/saved_objects/visualization/Second.json",
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
}

fn read_canonical_export(project: &TempDir, path: &Path) -> String {
    let canonical = std::fs::read_to_string(path).unwrap();
    assert!(
        !project
            .path()
            .join("kb/esdiag/saved_objects/Chart-obj-1.json")
            .exists()
    );
    assert!(canonical.contains("\"visState\": {"));
    assert!(canonical.contains("# keep\\nx: 1"));
    assert!(!canonical.contains("\"sort\""));
    assert!(
        !project
            .path()
            .join("kb/esdiag/saved_objects/export.ndjson")
            .exists()
    );
    canonical
}

#[test]
fn json_list_and_map_bundles_keep_their_declared_shape() {
    let fake = Fake::start();
    let project = setup(&fake.url);
    configure_bundle_inputs(&project);
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

fn configure_bundle_inputs(project: &TempDir) {
    edit_catalog(project, |definition| {
        for (name, shape) in [("map_items", "map"), ("list_items", "list")] {
            definition["resource_types"][name] = serde_yaml::to_value(json!([{
                "id": {"pointer": "/id", "scope": "universal"},
                "display_name": {"pointer": "/name", "strategy": "name"},
                "write_intent": "upsert",
                "metadata": {"fields": ["/created_at"]},
                "operations": {
                    "read": {"method": "GET", "path": format!("/api/{name}/{{id}}"), "cardinality": "one"},
                    "upsert": {
                        "method": "POST",
                        "path": "/api/saved_objects/_import",
                        "cardinality": "many",
                        "bundle": {"shape": shape, "format": "json"}
                    }
                }
            }]))
            .unwrap();
        }
    });
    std::fs::create_dir_all(project.path().join("kb")).unwrap();
    std::fs::write(
        project.path().join("kb/.target.yaml"),
        "schema_version: 1\nmetadata: { track: true }\n",
    )
    .unwrap();
    let directory = project.path().join("kb/map_items");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("Two.json"),
        serde_json::to_string_pretty(
            &json!({"id": "two", "name": "Two", "value": 2, "created_at": "server-owned"}),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.join("One.json"),
        serde_json::to_string_pretty(
            &json!({"id": "one", "name": "One", "value": 1, "created_at": "server-owned"}),
        )
        .unwrap(),
    )
    .unwrap();
    let list_directory = project.path().join("kb/list_items");
    std::fs::create_dir_all(&list_directory).unwrap();
    std::fs::write(
        list_directory.join("Only.json"),
        serde_json::to_string_pretty(
            &json!({"id": "only", "name": "Only", "value": 1, "created_at": "server-owned"}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn saved_object_filenames_use_the_first_available_display_name_pointer() {
    let fake = Fake::start();
    let project = setup(&fake.url);

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
            .join("kb/esdiag/saved_objects/tag/ESDiag-208ebb83.json")
            .is_file()
    );
}

#[test]
fn namespaced_resource_types_require_an_explicit_namespace_directory() {
    let project = setup("http://invalid");

    edit_catalog(&project, |definition| {
        definition["resource_types"]["saved_objects"][0]["namespaced"] =
            serde_yaml::Value::Bool(true);
    });

    write_namespace_examples(&project);

    let listed = run(&project, &["list"]);
    let resources = listed["result"].as_array().unwrap();
    assert_eq!(resources.len(), 3);
    for (namespace, id) in [
        ("esdiag", "dashboard-esdiag"),
        ("default", "dashboard-default"),
    ] {
        assert!(
            resources.iter().any(|resource| {
                resource["type"] == "saved_objects"
                    && resource["namespace"] == namespace
                    && resource["id"] == id
            }),
            "missing saved object {id} in {namespace}"
        );
    }
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

fn write_namespace_examples(project: &TempDir) {
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
}

#[test]
fn namespace_selectors_render_named_and_default_operation_paths() {
    let fake = Fake::start();
    let project = setup(&fake.url);

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
