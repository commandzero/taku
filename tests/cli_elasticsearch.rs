use assert_cmd::Command;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::process::Command as StdCommand;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Header, Response, Server};

#[derive(Clone, Debug)]
struct ScriptedResponse {
    status: u16,
    body: String,
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    body: String,
}

type ScriptedRoutes = Arc<Mutex<BTreeMap<(String, String), VecDeque<ScriptedResponse>>>>;

struct ScriptedApi {
    url: String,
    routes: ScriptedRoutes,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ScriptedApi {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let mut initial_routes = BTreeMap::<(String, String), VecDeque<ScriptedResponse>>::new();
        initial_routes.insert(
            ("GET".into(), "/".into()),
            VecDeque::from([ScriptedResponse {
                status: 200,
                body: r#"{"version":{"number":"9.1.0"}}"#.into(),
            }]),
        );
        let routes = Arc::new(Mutex::new(initial_routes));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let scripted = routes.clone();
        let captured = requests.clone();
        let stopping = stop.clone();
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let method = request.method().as_str().to_owned();
                let path = request.url().to_owned();
                captured.lock().unwrap().push(CapturedRequest {
                    method: method.clone(),
                    path: path.clone(),
                    body,
                });
                let response = {
                    let mut routes = scripted.lock().unwrap();
                    let queue = routes
                        .get_mut(&(method, path))
                        .expect("unexpected scripted API request");
                    if queue.len() > 1 {
                        queue.pop_front().unwrap()
                    } else {
                        queue.front().unwrap().clone()
                    }
                };
                request
                    .respond(
                        Response::from_string(response.body)
                            .with_status_code(response.status)
                            .with_header(
                                Header::from_bytes("content-type", "application/json").unwrap(),
                            ),
                    )
                    .unwrap();
            }
        });
        Self {
            url,
            routes,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn respond(&self, method: &str, path: &str, status: u16, body: &str) {
        self.routes
            .lock()
            .unwrap()
            .entry((method.into(), path.into()))
            .or_default()
            .push_back(ScriptedResponse {
                status,
                body: body.into(),
            });
    }
}

impl Drop for ScriptedApi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

struct FakeEnrichApi {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeEnrichApi {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Some(request) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                    continue;
                };
                let body = r#"{
  "policies": [
    {
      "config": {
        "match": {
          "name": "users",
          "indices": ["users"],
          "match_field": "email",
          "enrich_fields": ["full_name"],
          "elasticsearch_version": "9.1.0"
        }
      }
    },
    {
      "config": {
        "geo_match": {
          "name": "places",
          "indices": ["places"],
          "match_field": "location",
          "enrich_fields": ["region"],
          "elasticsearch_version": "9.1.0"
        }
      }
    }
  ]
}"#;
                let response_body = if request.url() == "/" {
                    r#"{"version":{"number":"9.1.0"}}"#
                } else {
                    body
                };
                let response = Response::from_string(response_body)
                    .with_header(Header::from_bytes("content-type", "application/json").unwrap());
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FakeEnrichApi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn project() -> TempDir {
    project_at("http://127.0.0.1:1")
}

fn project_at(url: &str) -> TempDir {
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
        &["init", "--layout", "single", "--environment", "test"],
    );
    run(&project, &["install", "elasticsearch"]);
    run(
        &project,
        &["target", "add", "elasticsearch", "es", "--url", url],
    );
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
        "taku {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

struct UpsertExercise<'a> {
    resource_type: &'a str,
    id: &'a str,
    list_path: &'a str,
    read_path: &'a str,
    put_path: &'a str,
    delete_path: &'a str,
    list_response: &'a str,
    read_response: &'a str,
    desired: Value,
    expected_wire: Value,
}

fn assert_upsert_lifecycle(exercise: UpsertExercise<'_>) {
    let api = ScriptedApi::start();
    api.respond("GET", exercise.list_path, 200, exercise.list_response);
    api.respond("GET", exercise.read_path, 200, exercise.read_response);
    api.respond("PUT", exercise.put_path, 200, r#"{"acknowledged":true}"#);
    api.respond(
        "DELETE",
        exercise.delete_path,
        200,
        r#"{"acknowledged":true}"#,
    );
    let project = project_at(&api.url);
    run(
        &project,
        &["add", "es", exercise.resource_type, exercise.id],
    );
    run(
        &project,
        &["fetch", "es", exercise.resource_type, exercise.id],
    );
    let resource = std::fs::read_dir(project.path().join("es").join(exercise.resource_type))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(
        &resource,
        serde_json::to_string_pretty(&exercise.desired).unwrap(),
    )
    .unwrap();
    run(
        &project,
        &[
            "push",
            "es",
            exercise.resource_type,
            exercise.id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    run(
        &project,
        &["fetch", "es", exercise.resource_type, exercise.id],
    );
    run(
        &project,
        &["remove", "es", exercise.resource_type, exercise.id],
    );
    run(
        &project,
        &[
            "push",
            "es",
            exercise.resource_type,
            exercise.id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );

    let requests = api.requests.lock().unwrap();
    let put = requests
        .iter()
        .find(|request| request.method == "PUT")
        .unwrap();
    assert_eq!(put.path, exercise.put_path);
    assert_eq!(
        serde_json::from_str::<Value>(&put.body).unwrap(),
        exercise.expected_wire
    );
    assert!(
        requests
            .iter()
            .any(|request| { request.method == "DELETE" && request.path == exercise.delete_path })
    );
}

#[test]
fn embedded_elasticsearch_catalog_recognizes_snapshot_repositories() {
    let project = project();
    let directory = project.path().join("es/snapshot_repositories");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("backups.json"),
        r#"{
  "id": "backups",
  "type": "fs",
  "settings": { "location": "backups" }
}"#,
    )
    .unwrap();

    let validated = run(&project, &["validate"]);

    assert_eq!(validated["result"]["valid"], true);
    let listed = run(&project, &["list", "es", "snapshot_repositories"]);
    assert_eq!(listed["result"][0]["id"], "backups");
}

#[test]
fn embedded_elasticsearch_catalog_recognizes_legacy_index_templates() {
    let project = project();
    let directory = project.path().join("es/legacy_index_templates");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("legacy-logs.json"),
        r#"{
  "id": "legacy-logs",
  "index_patterns": ["legacy-logs-*"],
  "order": 10,
  "settings": { "number_of_shards": 1 }
}"#,
    )
    .unwrap();

    let validated = run(&project, &["validate"]);

    assert_eq!(validated["result"]["valid"], true);
    let listed = run(&project, &["list", "es", "legacy_index_templates"]);
    assert_eq!(listed["result"][0]["id"], "legacy-logs");
}

#[test]
fn embedded_elasticsearch_catalog_recognizes_security_role_mappings() {
    let project = project();
    let directory = project.path().join("es/role_mappings");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("admins.json"),
        r#"{
  "id": "admins",
  "enabled": true,
  "roles": ["superuser"],
  "rules": { "field": { "groups": "cn=admins,dc=example,dc=com" } },
  "metadata": {}
}"#,
    )
    .unwrap();

    let validated = run(&project, &["validate"]);

    assert_eq!(validated["result"]["valid"], true);
    let listed = run(&project, &["list", "es", "role_mappings"]);
    assert_eq!(listed["result"][0]["id"], "admins");
}

#[test]
fn embedded_elasticsearch_catalog_recognizes_snapshot_lifecycle_policies() {
    let project = project();
    let directory = project.path().join("es/slm_policies");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("daily.json"),
        r#"{
  "id": "daily",
  "policy": {
    "schedule": "0 30 1 * * ?",
    "name": "<daily-{now/d}>",
    "repository": "backups",
    "config": { "indices": ["logs-*"] },
    "retention": { "expire_after": "30d", "min_count": 1 }
  }
}"#,
    )
    .unwrap();

    let validated = run(&project, &["validate"]);

    assert_eq!(validated["result"]["valid"], true);
    let listed = run(&project, &["list", "es", "slm_policies"]);
    assert_eq!(listed["result"][0]["id"], "daily");
}

#[test]
fn embedded_elasticsearch_catalog_recognizes_ccr_auto_follow_patterns() {
    let project = project();
    let directory = project.path().join("es/ccr_auto_follow_patterns");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("logs.json"),
        r#"{
  "name": "logs",
  "pattern": {
    "remote_cluster": "leader",
    "leader_index_patterns": ["logs-*"]
  }
}"#,
    )
    .unwrap();

    let validated = run(&project, &["validate"]);

    assert_eq!(validated["result"]["valid"], true);
    let listed = run(&project, &["list", "es", "ccr_auto_follow_patterns"]);
    assert_eq!(listed["result"][0]["id"], "logs");
}

#[test]
fn embedded_elasticsearch_catalog_recognizes_target_scoped_cluster_settings() {
    let project = project();
    let directory = project.path().join("es/cluster_settings");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("cluster-settings.json"),
        r#"{
  "id": "cluster-settings",
  "persistent": { "cluster.max_shards_per_node": "1000" },
  "transient": {}
}"#,
    )
    .unwrap();

    let validated = run(&project, &["validate"]);

    assert_eq!(validated["result"]["valid"], true);
    let listed = run(&project, &["list", "es", "cluster_settings"]);
    assert_eq!(listed["result"][0]["id"], "cluster-settings");
}

#[test]
fn enrich_policy_types_round_trip_from_dynamic_response_keys() {
    let api = FakeEnrichApi::start();
    let project = project_at(&api.url);

    let listed = run(&project, &["list", "--remote", "es", "enrich_policies"]);

    assert_eq!(listed["result"].as_array().unwrap().len(), 2);
    let mut ids: Vec<_> = listed["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, ["places", "users"]);
    run(&project, &["add", "es", "enrich_policies", "users"]);
    let resource = std::fs::read_to_string(
        std::fs::read_dir(project.path().join("es/enrich_policies"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
    )
    .unwrap();
    assert!(resource.contains(r#""policy_type": "match""#));
    assert!(resource.contains(r#""name": "users""#));
    assert!(!resource.contains("elasticsearch_version"));
}

#[test]
fn snapshot_repositories_use_normalized_put_and_delete_operations() {
    let api = ScriptedApi::start();
    api.respond(
        "GET",
        "/_snapshot/backups",
        200,
        r#"{"backups":{"type":"fs","uuid":"server-only","settings":{"location":"before"}}}"#,
    );
    api.respond(
        "GET",
        "/_snapshot",
        200,
        r#"{"backups":{"type":"fs","uuid":"server-only","settings":{"location":"before"}}}"#,
    );
    api.respond("PUT", "/_snapshot/backups", 200, r#"{"acknowledged":true}"#);
    api.respond(
        "DELETE",
        "/_snapshot/backups",
        200,
        r#"{"acknowledged":true}"#,
    );
    let project = project_at(&api.url);
    run(&project, &["add", "es", "snapshot_repositories", "backups"]);
    run(
        &project,
        &["fetch", "es", "snapshot_repositories", "backups"],
    );
    let resource = project.path().join("es/snapshot_repositories/backups.json");
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&resource).unwrap()).unwrap();
    assert!(value.get("uuid").is_none());
    value["settings"]["location"] = json!("after");
    std::fs::write(&resource, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    run(
        &project,
        &[
            "push",
            "es",
            "snapshot_repositories",
            "backups",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    run(
        &project,
        &["fetch", "es", "snapshot_repositories", "backups"],
    );
    run(
        &project,
        &["remove", "es", "snapshot_repositories", "backups"],
    );
    run(
        &project,
        &[
            "push",
            "es",
            "snapshot_repositories",
            "backups",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );

    let requests = api.requests.lock().unwrap();
    let put = requests
        .iter()
        .find(|request| request.method == "PUT")
        .unwrap();
    assert_eq!(put.path, "/_snapshot/backups");
    assert_eq!(
        serde_json::from_str::<Value>(&put.body).unwrap(),
        json!({"type":"fs","settings":{"location":"after"}})
    );
    assert!(
        requests
            .iter()
            .any(|request| { request.method == "DELETE" && request.path == "/_snapshot/backups" })
    );
}

#[test]
fn legacy_index_templates_use_update_and_delete_operations() {
    let response = r#"{"legacy-logs":{"index_patterns":["legacy-*"],"order":1,"settings":{}}}"#;
    assert_upsert_lifecycle(UpsertExercise {
        resource_type: "legacy_index_templates",
        id: "legacy-logs",
        list_path: "/_template",
        read_path: "/_template/legacy-logs",
        put_path: "/_template/legacy-logs",
        delete_path: "/_template/legacy-logs",
        list_response: response,
        read_response: response,
        desired: json!({
            "id": "legacy-logs",
            "index_patterns": ["legacy-logs-*"],
            "order": 2,
            "settings": {"number_of_shards": 1}
        }),
        expected_wire: json!({
            "index_patterns": ["legacy-logs-*"],
            "order": 2,
            "settings": {"number_of_shards": 1}
        }),
    });
}

#[test]
fn role_mappings_use_normalized_upsert_and_delete_operations() {
    let response = r#"{"admins":{"enabled":true,"roles":["viewer"],"rules":{"field":{"username":"*"}},"metadata":{}}}"#;
    assert_upsert_lifecycle(UpsertExercise {
        resource_type: "role_mappings",
        id: "admins",
        list_path: "/_security/role_mapping",
        read_path: "/_security/role_mapping/admins",
        put_path: "/_security/role_mapping/admins",
        delete_path: "/_security/role_mapping/admins",
        list_response: response,
        read_response: response,
        desired: json!({
            "id": "admins",
            "enabled": true,
            "roles": ["superuser"],
            "rules": {"field": {"groups": "cn=admins,dc=example,dc=com"}},
            "metadata": {"managed_by": "taku"}
        }),
        expected_wire: json!({
            "enabled": true,
            "roles": ["superuser"],
            "rules": {"field": {"groups": "cn=admins,dc=example,dc=com"}},
            "metadata": {"managed_by": "taku"}
        }),
    });
}

#[test]
fn slm_policies_drop_execution_metadata_and_write_only_the_policy() {
    let response = r#"{
  "daily": {
    "version": 7,
    "modified_date_millis": 123,
    "next_execution_millis": 456,
    "stats": {"snapshots_taken": 2},
    "policy": {
      "schedule": "0 30 1 * * ?",
      "name": "<daily-{now/d}>",
      "repository": "backups",
      "config": {"indices": ["logs-*"]}
    }
  }
}"#;
    let policy = json!({
        "schedule": "0 0 2 * * ?",
        "name": "<daily-{now/d}>",
        "repository": "backups",
        "config": {"indices": ["logs-*"]},
        "retention": {"expire_after": "30d"}
    });
    assert_upsert_lifecycle(UpsertExercise {
        resource_type: "slm_policies",
        id: "daily",
        list_path: "/_slm/policy",
        read_path: "/_slm/policy/daily",
        put_path: "/_slm/policy/daily",
        delete_path: "/_slm/policy/daily",
        list_response: response,
        read_response: response,
        desired: json!({"id": "daily", "policy": policy.clone()}),
        expected_wire: policy,
    });
}

#[test]
fn ilm_policies_drop_version_metadata_and_write_only_the_policy() {
    let response = r#"{
  "logs": {
    "version": 3,
    "modified_date": "2026-08-15T00:00:00Z",
    "in_use_by": {"indices": [], "data_streams": [], "composable_templates": []},
    "policy": {"phases": {"delete": {"min_age": "30d", "actions": {"delete": {}}}}}
  }
}"#;
    let policy = json!({
        "phases": {"delete": {"min_age": "14d", "actions": {"delete": {}}}}
    });
    assert_upsert_lifecycle(UpsertExercise {
        resource_type: "ilm_policies",
        id: "logs",
        list_path: "/_ilm/policy",
        read_path: "/_ilm/policy/logs",
        put_path: "/_ilm/policy/logs",
        delete_path: "/_ilm/policy/logs",
        list_response: response,
        read_response: response,
        desired: json!({"id": "logs", "policy": policy.clone()}),
        expected_wire: policy,
    });
}

#[test]
fn ccr_auto_follow_patterns_drop_active_state_and_write_only_the_pattern() {
    let response = r#"{
  "patterns": [{
    "name": "logs",
    "pattern": {
      "active": false,
      "remote_cluster": "leader",
      "leader_index_patterns": ["logs-*"]
    }
  }]
}"#;
    let pattern = json!({
        "remote_cluster": "leader",
        "leader_index_patterns": ["logs-prod-*"]
    });
    assert_upsert_lifecycle(UpsertExercise {
        resource_type: "ccr_auto_follow_patterns",
        id: "logs",
        list_path: "/_ccr/auto_follow",
        read_path: "/_ccr/auto_follow/logs",
        put_path: "/_ccr/auto_follow/logs",
        delete_path: "/_ccr/auto_follow/logs",
        list_response: response,
        read_response: response,
        desired: json!({"name": "logs", "pattern": pattern.clone()}),
        expected_wire: pattern,
    });
}

#[test]
fn cluster_settings_use_a_target_scoped_update_operation() {
    let api = ScriptedApi::start();
    api.respond(
        "GET",
        "/_cluster/settings?flat_settings=true",
        200,
        r#"{"persistent":{"cluster.max_shards_per_node":"1000"},"transient":{}}"#,
    );
    api.respond("PUT", "/_cluster/settings", 200, r#"{"acknowledged":true}"#);
    let project = project_at(&api.url);
    let directory = project.path().join("es/cluster_settings");
    std::fs::create_dir_all(&directory).unwrap();
    let resource = directory.join("cluster-settings.json");
    std::fs::write(
        &resource,
        serde_json::to_string_pretty(&json!({
            "id": "cluster-settings",
            "persistent": {"cluster.max_shards_per_node": "1000"},
            "transient": {}
        }))
        .unwrap(),
    )
    .unwrap();
    run(
        &project,
        &["fetch", "es", "cluster_settings", "cluster-settings"],
    );
    let mut desired: Value =
        serde_json::from_str(&std::fs::read_to_string(&resource).unwrap()).unwrap();
    desired["persistent"]["cluster.max_shards_per_node"] = json!("1200");
    std::fs::write(&resource, serde_json::to_string_pretty(&desired).unwrap()).unwrap();

    run(
        &project,
        &[
            "push",
            "es",
            "cluster_settings",
            "cluster-settings",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );

    let requests = api.requests.lock().unwrap();
    let put = requests
        .iter()
        .find(|request| request.method == "PUT")
        .unwrap();
    assert_eq!(put.path, "/_cluster/settings");
    assert_eq!(
        serde_json::from_str::<Value>(&put.body).unwrap(),
        json!({
            "persistent": {"cluster.max_shards_per_node": "1200"},
            "transient": {}
        })
    );
}

#[test]
fn enrich_policies_create_from_canonical_type_and_delete_by_name() {
    let api = ScriptedApi::start();
    for _ in 0..2 {
        api.respond("GET", "/_enrich/policy/users", 404, r#"{"policies":[]}"#);
    }
    let remote = r#"{
  "policies": [{
    "config": {
      "match": {
        "name": "users",
        "indices": ["users"],
        "match_field": "email",
        "enrich_fields": ["full_name"],
        "elasticsearch_version": "9.1.0"
      }
    }
  }]
}"#;
    api.respond("GET", "/_enrich/policy/users", 200, remote);
    api.respond("GET", "/_enrich/policy/users", 200, remote);
    api.respond(
        "PUT",
        "/_enrich/policy/users",
        200,
        r#"{"acknowledged":true}"#,
    );
    api.respond(
        "DELETE",
        "/_enrich/policy/users",
        200,
        r#"{"acknowledged":true}"#,
    );
    let project = project_at(&api.url);
    let directory = project.path().join("es/enrich_policies");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("users.json"),
        serde_json::to_string_pretty(&json!({
            "policy_type": "match",
            "policy": {
                "name": "users",
                "indices": ["users"],
                "match_field": "email",
                "enrich_fields": ["full_name"]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    run(&project, &["fetch", "es", "enrich_policies", "users"]);
    run(
        &project,
        &[
            "push",
            "es",
            "enrich_policies",
            "users",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    run(&project, &["fetch", "es", "enrich_policies", "users"]);
    run(&project, &["remove", "es", "enrich_policies", "users"]);
    run(
        &project,
        &[
            "push",
            "es",
            "enrich_policies",
            "users",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );

    let requests = api.requests.lock().unwrap();
    let put = requests
        .iter()
        .find(|request| request.method == "PUT")
        .unwrap();
    assert_eq!(put.path, "/_enrich/policy/users");
    assert_eq!(
        serde_json::from_str::<Value>(&put.body).unwrap(),
        json!({
            "match": {
                "indices": ["users"],
                "match_field": "email",
                "enrich_fields": ["full_name"]
            }
        })
    );
    assert!(
        requests.iter().any(|request| {
            request.method == "DELETE" && request.path == "/_enrich/policy/users"
        })
    );
}
