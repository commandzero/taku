use assert_cmd::Command;
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

#[path = "live_elastic/corpus.rs"]
mod corpus;

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
    run_at(project.path(), authorization, args)
}

fn run_at(project: &std::path::Path, authorization: &str, args: &[&str]) -> Value {
    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project)
        .env(AUTH_ENV, authorization)
        .env("ELASTIC_AUTHORIZATION", authorization)
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

#[derive(Clone, Copy)]
enum Service {
    Elasticsearch,
    Kibana,
}

impl Service {
    fn config(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Elasticsearch => ("elasticsearch", "es", ELASTICSEARCH_URL),
            Self::Kibana => ("kibana", "kb", KIBANA_URL),
        }
    }
}

fn project(service: Service, authorization: &str) -> TempDir {
    let (application, target, url) = service.config();
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
        &["target", "add", application, target, "--url", url],
    );

    let path = project.path().join(".taku/project.yaml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["environments"]["live"]["targets"][target]["auth"] =
        serde_yaml::from_str(&format!("fields:\n  authorization: {AUTH_ENV}\n")).unwrap();
    std::fs::write(path, serde_yaml::to_string(&config).unwrap()).unwrap();
    project
}

struct LiveResource<'a> {
    project: &'a TempDir,
    authorization: &'a str,
    target: &'static str,
    kind: &'static str,
    id: &'a str,
}

impl LiveResource<'_> {
    fn run(&self, command: &str, options: &[&str]) -> Value {
        let mut args = vec![command];
        args.extend_from_slice(options);
        args.extend([self.target, self.kind, self.id]);
        run(self.project, self.authorization, &args)
    }

    fn push(&self) -> Value {
        run(
            self.project,
            self.authorization,
            &[
                "push",
                self.target,
                self.kind,
                self.id,
                "--uncommitted",
                "allow",
                "--untracked",
                "allow",
            ],
        )
    }

    fn first_file(&self) -> PathBuf {
        std::fs::read_dir(self.project.path().join(self.target).join(self.kind))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
    }

    fn assert_in_sync(&self) {
        self.run("fetch", &[]);
        let status = self.run("status", &[]);
        assert_eq!(status["result"][0]["state"], "in_sync");
    }

    fn assert_deleted(&self) {
        self.run("remove", &[]);
        let deleted = self.push();
        assert_eq!(deleted["result"][0]["outcome"], "deleted");
    }
}

fn assert_fixture_created(request: reqwest::blocking::RequestBuilder, body: &Value) {
    let response = request.json(body).send().unwrap();
    assert!(response.status().is_success(), "fixture creation failed");
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn manages_an_elasticsearch_ingest_pipeline_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("pipeline");
    let client = Client::new();
    let url = format!("{ELASTICSEARCH_URL}/_ingest/pipeline/{id}");
    assert_fixture_created(
        client.put(&url).header(AUTHORIZATION, &authorization),
        &json!({
            "description": "created by the Taku live integration test",
            "processors": []
        }),
    );
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: url,
        kibana: false,
    };
    let project = project(Service::Elasticsearch, &authorization);
    let resource = LiveResource {
        project: &project,
        authorization: &authorization,
        target: "es",
        kind: "ingest_pipelines",
        id: &id,
    };

    let listed = resource.run("list", &["--remote"]);
    assert_eq!(listed["result"][0]["id"], id);
    resource.run("add", &[]);
    resource.run("fetch", &[]);

    let path = resource.first_file();
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(value.get("created_date_millis").is_none());
    assert!(value.get("modified_date_millis").is_none());
    value["description"] = json!("updated through Taku");
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let pushed = resource.push();
    assert_eq!(pushed["result"][0]["outcome"], "success");
    resource.assert_in_sync();

    resource.assert_deleted();
    let listed = resource.run("list", &["--remote"]);
    assert_eq!(listed["result"], json!([]));

    drop(fixture);
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn round_trips_a_flat_component_template_through_fetch_pull_and_push() {
    let authorization = authorization();
    let id = unique_id("component-template");
    let client = Client::new();
    let url = format!("{ELASTICSEARCH_URL}/_component_template/{id}");
    let create_body = json!({
        "version": 1,
        "_meta": {"description": "initial"},
        "template": {"settings": {"number_of_shards": 1}}
    });
    assert_fixture_created(
        client.put(&url).header(AUTHORIZATION, &authorization),
        &create_body,
    );
    let fixture = RemoteFixture {
        client: client.clone(),
        authorization: authorization.clone(),
        delete_url: url.clone(),
        kibana: false,
    };
    let project = project(Service::Elasticsearch, &authorization);
    let resource = LiveResource {
        project: &project,
        authorization: &authorization,
        target: "es",
        kind: "component_templates",
        id: &id,
    };

    resource.run("add", &[]);
    resource.run("fetch", &[]);
    let path = resource.first_file();
    let initial: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(initial["_taku"]["id"], id);
    assert!(initial.get("name").is_none());
    assert_eq!(initial["_meta"]["description"], "initial");
    assert!(initial.get("component_template").is_none());

    let response = client
        .put(&url)
        .header(AUTHORIZATION, &authorization)
        .json(&json!({
            "version": 1,
            "_meta": {"description": "remote"},
            "template": {"settings": {"number_of_shards": 1}}
        }))
        .send()
        .unwrap();
    assert!(response.status().is_success(), "remote update failed");
    resource.run("fetch", &[]);
    resource.run("pull", &["--yes"]);
    let mut canonical: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(canonical["_meta"]["description"], "remote");
    canonical["_meta"]["description"] = json!("local");
    std::fs::write(&path, serde_json::to_string_pretty(&canonical).unwrap()).unwrap();
    resource.push();
    let remote: Value = client
        .get(&url)
        .header(AUTHORIZATION, &authorization)
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(
        remote["component_templates"][0]["component_template"]["_meta"]["description"],
        "local"
    );

    drop(fixture);
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn tracks_elasticsearch_metadata_with_a_resource_type_hint() {
    let authorization = authorization();
    let id = unique_id("metadata-pipeline");
    let client = Client::new();
    let url = format!("{ELASTICSEARCH_URL}/_ingest/pipeline/{id}");
    assert!(
        client
            .put(&url)
            .header(AUTHORIZATION, &authorization)
            .json(&json!({"description": "metadata fixture", "processors": []}))
            .send()
            .unwrap()
            .status()
            .is_success()
    );
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: url,
        kibana: false,
    };
    let project = project(Service::Elasticsearch, &authorization);
    let resource = LiveResource {
        project: &project,
        authorization: &authorization,
        target: "es",
        kind: "ingest_pipelines",
        id: &id,
    };
    let directory = project.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(".resource.yaml"),
        "schema_version: 1\nmetadata: { track: true }\n",
    )
    .unwrap();

    resource.run("add", &[]);
    let resource = std::fs::read_dir(&directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.file_name().and_then(|name| name.to_str()) != Some(".resource.yaml"))
        .unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(resource).unwrap()).unwrap();
    assert!(value.get("created_date_millis").is_some());
    assert!(value.get("modified_date_millis").is_some());

    drop(fixture);
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn reads_all_declarative_elasticsearch_resource_types_from_real_apis() {
    let authorization = authorization();
    let project = project(Service::Elasticsearch, &authorization);

    for resource_type in [
        "snapshot_repositories",
        "legacy_index_templates",
        "role_mappings",
        "slm_policies",
        "ccr_auto_follow_patterns",
        "enrich_policies",
    ] {
        let listed = run(
            &project,
            &authorization,
            &["list", "--remote", "es", resource_type],
        );
        assert!(listed["result"].is_array(), "{resource_type}");
    }

    let settings_directory = project.path().join("es/cluster_settings");
    std::fs::create_dir_all(&settings_directory).unwrap();
    std::fs::write(
        settings_directory.join("cluster-settings.json"),
        r#"{"id":"cluster-settings","persistent":{},"transient":{}}"#,
    )
    .unwrap();
    let settings = run(
        &project,
        &authorization,
        &["fetch", "es", "cluster_settings", "cluster-settings"],
    );
    assert_eq!(settings["result"][0]["outcome"], "observed");
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn round_trips_a_legacy_index_template_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("legacy-template");
    let client = Client::new();
    let url = format!("{ELASTICSEARCH_URL}/_template/{id}");
    assert_fixture_created(
        client.put(&url).header(AUTHORIZATION, &authorization),
        &json!({
            "index_patterns": [format!("{id}-*")],
            "order": 1,
            "settings": {"number_of_shards": 1}
        }),
    );
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: url,
        kibana: false,
    };
    let project = project(Service::Elasticsearch, &authorization);
    let resource = LiveResource {
        project: &project,
        authorization: &authorization,
        target: "es",
        kind: "legacy_index_templates",
        id: &id,
    };

    resource.run("add", &[]);
    resource.run("fetch", &[]);
    let path = resource.first_file();
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["order"] = json!(2);
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    resource.push();
    resource.assert_in_sync();
    resource.assert_deleted();

    drop(fixture);
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn round_trips_a_role_mapping_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("role-mapping");
    let client = Client::new();
    let url = format!("{ELASTICSEARCH_URL}/_security/role_mapping/{id}");
    assert_fixture_created(
        client.put(&url).header(AUTHORIZATION, &authorization),
        &json!({
            "enabled": true,
            "roles": ["viewer"],
            "rules": {"field": {"username": "taku-live-*"}},
            "metadata": {"test": true}
        }),
    );
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: url,
        kibana: false,
    };
    let project = project(Service::Elasticsearch, &authorization);
    let resource = LiveResource {
        project: &project,
        authorization: &authorization,
        target: "es",
        kind: "role_mappings",
        id: &id,
    };

    resource.run("add", &[]);
    resource.run("fetch", &[]);
    let path = resource.first_file();
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["roles"] = json!(["monitoring_user"]);
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    resource.push();
    resource.assert_in_sync();
    resource.assert_deleted();

    drop(fixture);
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn creates_and_deletes_an_enrich_policy_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("enrich-policy");
    let project = project(Service::Elasticsearch, &authorization);
    let resource = LiveResource {
        project: &project,
        authorization: &authorization,
        target: "es",
        kind: "enrich_policies",
        id: &id,
    };
    let directory = project.path().join("es/enrich_policies");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{id}.json")),
        serde_json::to_string_pretty(&json!({
            "policy_type": "match",
            "policy": {
                "name": id,
                "indices": ["taku-live-enrich-source-*"],
                "match_field": "email",
                "enrich_fields": ["full_name"]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    resource.run("fetch", &[]);
    let created = resource.push();
    assert_eq!(created["result"][0]["outcome"], "success");
    let client = Client::new();
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: format!("{ELASTICSEARCH_URL}/_enrich/policy/{id}"),
        kibana: false,
    };
    resource.assert_in_sync();
    resource.assert_deleted();

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
    let project = project(Service::Kibana, &authorization);
    let resource = LiveResource {
        project: &project,
        authorization: &authorization,
        target: "kb",
        kind: "spaces",
        id: &id,
    };

    let listed = resource.run("list", &["--remote"]);
    assert_eq!(listed["result"][0]["id"], id);
    resource.run("add", &[]);
    resource.run("fetch", &[]);

    let path = resource.first_file();
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["description"] = json!("updated through Taku");
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let pushed = resource.push();
    assert_eq!(pushed["result"][0]["outcome"], "success");
    resource.assert_in_sync();

    resource.assert_deleted();
    let listed = resource.run("list", &["--remote"]);
    assert_eq!(listed["result"], json!([]));

    drop(fixture);
}

#[test]
#[ignore = "requires Kibana on localhost:5601 and ELASTIC_API_KEY in .env"]
fn lists_real_kibana_dashboard_exports_without_treating_export_details_as_a_resource() {
    let authorization = authorization();
    let project = project(Service::Kibana, &authorization);
    let definition_path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    definition["resource_types"]["saved_objects"][0]["operations"]["list"]["body"]["type"] =
        serde_yaml::to_value(["dashboard"]).unwrap();
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();

    let listed = run(
        &project,
        &authorization,
        &[
            "list",
            "--remote",
            "--namespace",
            "default",
            "kb",
            "saved_objects",
        ],
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

#[test]
#[ignore = "requires Kibana 9.4+ with Agent Builder on localhost:5601 and ELASTIC_API_KEY in .env"]
fn lists_agent_builder_plugins_through_the_real_kibana_api() {
    let authorization = authorization();
    let project = project(Service::Kibana, &authorization);

    let listed = run(
        &project,
        &authorization,
        &[
            "list",
            "--remote",
            "--namespace",
            "default",
            "kb",
            "plugins",
        ],
    );

    assert!(listed["result"].is_array());
}
