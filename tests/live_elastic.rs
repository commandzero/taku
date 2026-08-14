use assert_cmd::Command;
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use serde_json::{Value, json};
use std::process::Command as StdCommand;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

const ELASTICSEARCH_URL: &str = "http://localhost:9200";
const KIBANA_URL: &str = "http://localhost:5601";
const AUTH_ENV: &str = "TAKU_LIVE_AUTHORIZATION";

struct RemoteFixture {
    client: Client,
    authorization: String,
    delete_url: String,
    kibana: bool,
}

impl Drop for RemoteFixture {
    fn drop(&mut self) {
        let mut request = self
            .client
            .delete(&self.delete_url)
            .header(AUTHORIZATION, &self.authorization);
        if self.kibana {
            request = request.header("kbn-xsrf", "taku-live-test");
        }
        let _ = request.send();
    }
}

fn authorization() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    let key = dotenvy::from_path_iter(path)
        .expect("the repository .env file must be readable")
        .find_map(|entry| {
            let (name, value) = entry.expect("the repository .env file must be valid");
            (name == "ELASTIC_API_KEY").then_some(value)
        })
        .expect("ELASTIC_API_KEY must be defined in the repository .env file");
    if key.starts_with("ApiKey ") {
        key
    } else {
        format!("ApiKey {key}")
    }
}

fn unique_id(kind: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("taku-live-{kind}-{}-{nanos}", std::process::id())
}

fn run(project: &TempDir, authorization: &str, args: &[&str]) -> Value {
    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .env(AUTH_ENV, authorization)
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

fn project(application: &str, target: &str, url: &str, authorization: &str) -> TempDir {
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
        authorization,
        &["init", "--layout", "single", "--environment", "live"],
    );
    run(&project, authorization, &["install", application]);
    run(
        &project,
        authorization,
        &["app", "add", application, target, "--url", url],
    );

    let path = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["environments"]["live"]["targets"][target]["auth"] =
        serde_yaml::from_str(&format!("fields:\n  authorization: {AUTH_ENV}\n")).unwrap();
    std::fs::write(path, serde_yaml::to_string(&config).unwrap()).unwrap();
    project
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn manages_an_elasticsearch_ingest_pipeline_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("pipeline");
    let client = Client::new();
    let url = format!("{ELASTICSEARCH_URL}/_ingest/pipeline/{id}");
    let response = client
        .put(&url)
        .header(AUTHORIZATION, &authorization)
        .json(&json!({
            "description": "created by the Taku live integration test",
            "processors": []
        }))
        .send()
        .unwrap();
    assert!(response.status().is_success(), "fixture creation failed");
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: url,
        kibana: false,
    };
    let project = project("elasticsearch", "es", ELASTICSEARCH_URL, &authorization);

    let listed = run(
        &project,
        &authorization,
        &[
            "list",
            "--remote",
            "--type",
            "ingest_pipelines",
            "--id",
            &id,
        ],
    );
    assert_eq!(listed["result"][0]["id"], id);
    run(
        &project,
        &authorization,
        &["add", "--type", "ingest_pipelines", "--id", &id],
    );
    run(
        &project,
        &authorization,
        &["fetch", "--type", "ingest_pipelines", "--id", &id],
    );

    let resource = std::fs::read_dir(project.path().join("es/ingest_pipelines"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&resource).unwrap()).unwrap();
    assert!(value.get("created_date_millis").is_none());
    assert!(value.get("modified_date_millis").is_none());
    value["description"] = json!("updated through Taku");
    std::fs::write(&resource, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let pushed = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "ingest_pipelines",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(pushed["result"][0]["outcome"], "success");
    run(
        &project,
        &authorization,
        &["fetch", "--type", "ingest_pipelines", "--id", &id],
    );
    let status = run(
        &project,
        &authorization,
        &["status", "--type", "ingest_pipelines", "--id", &id],
    );
    assert_eq!(status["result"][0]["state"], "in_sync");

    run(
        &project,
        &authorization,
        &["remove", "--type", "ingest_pipelines", "--id", &id],
    );
    let deleted = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "ingest_pipelines",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(deleted["result"][0]["outcome"], "deleted");
    let listed = run(
        &project,
        &authorization,
        &[
            "list",
            "--remote",
            "--type",
            "ingest_pipelines",
            "--id",
            &id,
        ],
    );
    assert_eq!(listed["result"], json!([]));

    drop(fixture);
}

#[test]
#[ignore = "requires Kibana on localhost:5601 and ELASTIC_API_KEY in .env"]
fn manages_a_kibana_space_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("space");
    let client = Client::new();
    let collection_url = format!("{KIBANA_URL}/api/spaces/space");
    let response = client
        .post(&collection_url)
        .header(AUTHORIZATION, &authorization)
        .header("kbn-xsrf", "taku-live-test")
        .json(&json!({
            "id": id,
            "name": "Taku live test",
            "description": "created by the Taku live integration test",
            "initials": "TT",
            "color": "#00BFB3",
            "disabledFeatures": []
        }))
        .send()
        .unwrap();
    assert!(response.status().is_success(), "fixture creation failed");
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: format!("{collection_url}/{id}"),
        kibana: true,
    };
    let project = project("kibana", "kb", KIBANA_URL, &authorization);

    let listed = run(
        &project,
        &authorization,
        &["list", "--remote", "--type", "spaces", "--id", &id],
    );
    assert_eq!(listed["result"][0]["id"], id);
    run(
        &project,
        &authorization,
        &["add", "--type", "spaces", "--id", &id],
    );
    run(
        &project,
        &authorization,
        &["fetch", "--type", "spaces", "--id", &id],
    );

    let resource = std::fs::read_dir(project.path().join("kb/spaces"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&resource).unwrap()).unwrap();
    value["description"] = json!("updated through Taku");
    std::fs::write(&resource, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let pushed = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "spaces",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(pushed["result"][0]["outcome"], "success");
    run(
        &project,
        &authorization,
        &["fetch", "--type", "spaces", "--id", &id],
    );
    let status = run(
        &project,
        &authorization,
        &["status", "--type", "spaces", "--id", &id],
    );
    assert_eq!(status["result"][0]["state"], "in_sync");

    run(
        &project,
        &authorization,
        &["remove", "--type", "spaces", "--id", &id],
    );
    let deleted = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "spaces",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(deleted["result"][0]["outcome"], "deleted");
    let listed = run(
        &project,
        &authorization,
        &["list", "--remote", "--type", "spaces", "--id", &id],
    );
    assert_eq!(listed["result"], json!([]));

    drop(fixture);
}

#[test]
#[ignore = "requires Kibana on localhost:5601 and ELASTIC_API_KEY in .env"]
fn lists_real_kibana_dashboard_exports_without_treating_export_details_as_a_resource() {
    let authorization = authorization();
    let project = project("kibana", "kb", KIBANA_URL, &authorization);
    let definition_path = project
        .path()
        .join(".taku/applications/kibana/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["saved_objects"]["operations"]["list"]["body"]
        ["type"] = serde_yaml::to_value(["dashboard"]).unwrap();
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();

    let listed = run(
        &project,
        &authorization,
        &["list", "--remote", "--type", "saved_objects"],
    );

    let resources = listed["result"].as_array().unwrap();
    assert!(
        resources
            .iter()
            .all(|resource| resource.get("id").is_some())
    );
    assert!(
        resources
            .iter()
            .all(|resource| resource.get("type").is_some())
    );
}
