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

fn directory_files(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn visit(
        root: &std::path::Path,
        directory: &std::path::Path,
        files: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = std::collections::BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn skill_parts(bytes: &[u8]) -> (serde_yaml::Value, String) {
    let markdown = std::str::from_utf8(bytes).unwrap();
    let rest = markdown.strip_prefix("---\n").unwrap();
    let (frontmatter, body) = rest.split_once("\n---\n").unwrap();
    (serde_yaml::from_str(frontmatter).unwrap(), body.to_owned())
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
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn reads_all_declarative_elasticsearch_resource_types_from_real_apis() {
    let authorization = authorization();
    let project = project("elasticsearch", "es", ELASTICSEARCH_URL, &authorization);

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
            &["list", "--remote", "--type", resource_type],
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
        &[
            "fetch",
            "--type",
            "cluster_settings",
            "--id",
            "cluster-settings",
        ],
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
    let response = client
        .put(&url)
        .header(AUTHORIZATION, &authorization)
        .json(&json!({
            "index_patterns": [format!("{id}-*")],
            "order": 1,
            "settings": {"number_of_shards": 1}
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

    run(
        &project,
        &authorization,
        &["add", "--type", "legacy_index_templates", "--id", &id],
    );
    run(
        &project,
        &authorization,
        &["fetch", "--type", "legacy_index_templates", "--id", &id],
    );
    let resource = std::fs::read_dir(project.path().join("es/legacy_index_templates"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&resource).unwrap()).unwrap();
    value["order"] = json!(2);
    std::fs::write(&resource, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "legacy_index_templates",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    run(
        &project,
        &authorization,
        &["fetch", "--type", "legacy_index_templates", "--id", &id],
    );
    let status = run(
        &project,
        &authorization,
        &["status", "--type", "legacy_index_templates", "--id", &id],
    );
    assert_eq!(status["result"][0]["state"], "in_sync");
    run(
        &project,
        &authorization,
        &["remove", "--type", "legacy_index_templates", "--id", &id],
    );
    let deleted = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "legacy_index_templates",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(deleted["result"][0]["outcome"], "deleted");

    drop(fixture);
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn round_trips_a_role_mapping_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("role-mapping");
    let client = Client::new();
    let url = format!("{ELASTICSEARCH_URL}/_security/role_mapping/{id}");
    let response = client
        .put(&url)
        .header(AUTHORIZATION, &authorization)
        .json(&json!({
            "enabled": true,
            "roles": ["viewer"],
            "rules": {"field": {"username": "taku-live-*"}},
            "metadata": {"test": true}
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

    run(
        &project,
        &authorization,
        &["add", "--type", "role_mappings", "--id", &id],
    );
    run(
        &project,
        &authorization,
        &["fetch", "--type", "role_mappings", "--id", &id],
    );
    let resource = std::fs::read_dir(project.path().join("es/role_mappings"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut value: Value =
        serde_json::from_str(&std::fs::read_to_string(&resource).unwrap()).unwrap();
    value["roles"] = json!(["monitoring_user"]);
    std::fs::write(&resource, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "role_mappings",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    run(
        &project,
        &authorization,
        &["fetch", "--type", "role_mappings", "--id", &id],
    );
    let status = run(
        &project,
        &authorization,
        &["status", "--type", "role_mappings", "--id", &id],
    );
    assert_eq!(status["result"][0]["state"], "in_sync");
    run(
        &project,
        &authorization,
        &["remove", "--type", "role_mappings", "--id", &id],
    );
    let deleted = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "role_mappings",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(deleted["result"][0]["outcome"], "deleted");

    drop(fixture);
}

#[test]
#[ignore = "requires Elasticsearch on localhost:9200 and ELASTIC_API_KEY in .env"]
fn creates_and_deletes_an_enrich_policy_through_its_real_api() {
    let authorization = authorization();
    let id = unique_id("enrich-policy");
    let project = project("elasticsearch", "es", ELASTICSEARCH_URL, &authorization);
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
    run(
        &project,
        &authorization,
        &["fetch", "--type", "enrich_policies", "--id", &id],
    );
    let created = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "enrich_policies",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(created["result"][0]["outcome"], "success");
    let client = Client::new();
    let fixture = RemoteFixture {
        client,
        authorization: authorization.clone(),
        delete_url: format!("{ELASTICSEARCH_URL}/_enrich/policy/{id}"),
        kibana: false,
    };
    run(
        &project,
        &authorization,
        &["fetch", "--type", "enrich_policies", "--id", &id],
    );
    let status = run(
        &project,
        &authorization,
        &["status", "--type", "enrich_policies", "--id", &id],
    );
    assert_eq!(status["result"][0]["state"], "in_sync");
    run(
        &project,
        &authorization,
        &["remove", "--type", "enrich_policies", "--id", &id],
    );
    let deleted = run(
        &project,
        &authorization,
        &[
            "push",
            "--type",
            "enrich_policies",
            "--id",
            &id,
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(deleted["result"][0]["outcome"], "deleted");

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
        .join(".taku/applications/kibana/version-9.yml");
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
            "--type",
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
    let project = project("kibana", "kb", KIBANA_URL, &authorization);

    let listed = run(
        &project,
        &authorization,
        &[
            "list",
            "--remote",
            "--namespace",
            "default",
            "--type",
            "plugins",
        ],
    );

    assert!(listed["result"].is_array());
}

#[test]
#[ignore = "requires the ephemeral taku-test-project plus Elasticsearch and Kibana on localhost"]
fn round_trips_the_complete_esdiag_resource_corpus() {
    let authorization = authorization();
    let project = std::env::var_os("TAKU_TEST_PROJECT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("taku-test-project")
        });
    assert!(
        project.join(".git").exists(),
        "ephemeral test project is missing"
    );
    assert!(
        project
            .join("esdiag-assets/elasticsearch/assets.yml")
            .exists(),
        "the copied ESDiag source corpus is missing"
    );
    let skill_directory = project.join("kb/esdiag/skills/agentic-diagnostic-assistant");
    if !skill_directory.join("SKILL.md").is_file() {
        run_at(
            &project,
            &authorization,
            &[
                "add",
                "--target",
                "kb",
                "--namespace",
                "esdiag",
                "--type",
                "skills",
                "--id",
                "agentic-diagnostic-assistant",
            ],
        );
    }

    let validated = run_at(&project, &authorization, &["validate"]);
    assert_eq!(validated["result"]["valid"], true);

    let inventory = run_at(&project, &authorization, &["list"]);
    let resources = inventory["result"].as_array().unwrap();
    assert_eq!(resources.len(), 130);
    assert_eq!(
        resources
            .iter()
            .filter(|resource| resource["type"] == "saved_objects")
            .count(),
        90
    );
    assert!(resources.iter().any(|resource| {
        resource["target"] == "kb"
            && resource["namespace"] == "esdiag"
            && resource["type"] == "workflows"
    }));
    assert!(resources.iter().any(|resource| {
        resource["target"] == "kb"
            && resource.get("namespace").is_none()
            && resource["type"] == "spaces"
    }));
    let skill = resources
        .iter()
        .find(|resource| {
            resource["target"] == "kb"
                && resource["namespace"] == "esdiag"
                && resource["type"] == "skills"
        })
        .expect("the ESDiag Skill should be managed");
    let skill_path = skill["path"].as_str().unwrap();
    assert!(project.join(skill_path).join("SKILL.md").is_file());

    run_at(&project, &authorization, &["--target", "es", "fetch"]);
    run_at(&project, &authorization, &["--target", "kb", "fetch"]);
    run_at(&project, &authorization, &["pull", "--yes"]);

    let source_skill =
        project.join("esdiag-assets/kibana/esdiag/skills/agentic-diagnostic-assistant");
    let mut source_files = directory_files(&source_skill);
    let mut projected_files = directory_files(&project.join(skill_path));
    let source_document = source_files.remove("SKILL.md").unwrap();
    let projected_document = projected_files.remove("SKILL.md").unwrap();
    assert_eq!(
        source_files, projected_files,
        "Kibana Skill referenced files must round trip exactly"
    );
    assert_eq!(
        skill_parts(&source_document),
        skill_parts(&projected_document),
        "Kibana Skill frontmatter values and Markdown body must round trip"
    );

    let status = run_at(&project, &authorization, &["status"]);
    assert_eq!(status["result"].as_array().unwrap().len(), 130);
    assert!(
        status["result"]
            .as_array()
            .unwrap()
            .iter()
            .all(|resource| resource["state"] == "in_sync")
    );

    let push = run_at(
        &project,
        &authorization,
        &[
            "push",
            "--dry-run",
            "--uncommitted",
            "allow",
            "--untracked",
            "allow",
        ],
    );
    assert_eq!(push["result"].as_array().unwrap().len(), 130);
    assert!(
        push["result"]
            .as_array()
            .unwrap()
            .iter()
            .all(|resource| resource["outcome"] == "in_sync")
    );
}
