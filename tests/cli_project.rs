use assert_cmd::Command;
use serde_json::Value;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn project(layout: &str, environments: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        StdCommand::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let mut command = Command::cargo_bin("taku").unwrap();
    command
        .current_dir(dir.path())
        .args(["init", "--layout", layout]);
    if environments.len() == 1 {
        command.args(["--environment", environments[0]]);
    } else {
        command.arg("--environments").arg(environments.join(","));
    }
    command.assert().success();
    dir
}

fn json(project: &TempDir, args: &[&str]) -> Value {
    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .arg("--output")
        .arg("json")
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

fn failure(project: &TempDir, args: &[&str]) -> std::process::Output {
    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.code().is_some_and(|code| code != 0));
    output
}

#[test]
fn lists_and_installs_embedded_applications_without_adding_targets() {
    let project = project("single", &["dev"]);
    let listed = json(&project, &["app"]);
    assert_eq!(listed["result"][0]["name"], "elasticsearch");
    assert_eq!(listed["result"][1]["name"], "kibana");
    assert_eq!(listed["result"][0]["installed"], false);

    let installed = json(&project, &["install", "elasticsearch", "kibana"]);
    assert_eq!(installed["result"][0]["source"], "embedded");
    assert!(
        project
            .path()
            .join(".taku/applications/elasticsearch/resources.yml")
            .is_file()
    );
    assert!(
        project
            .path()
            .join(".taku/applications/kibana/resources.yml")
            .is_file()
    );

    let metadata = std::fs::read_to_string(project.path().join(".taku/project.yml")).unwrap();
    assert!(!metadata.contains("targets:\n    elasticsearch:"));
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["install", "elasticsearch"])
        .assert()
        .failure();
}

#[test]
fn adds_and_renames_environment_scoped_targets() {
    let project = project("multi", &["dev", "prod"]);
    json(&project, &["install", "elasticsearch"]);
    json(&project, &["context", "set", "dev"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es1",
            "--url",
            "http://127.0.0.1:9200",
        ],
    );
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es2",
            "--url",
            "http://127.0.0.1:9201",
        ],
    );
    json(&project, &["app", "rename", "es1", "cluster"]);

    let metadata = std::fs::read_to_string(project.path().join(".taku/project.yml")).unwrap();
    assert!(metadata.contains("cluster:"));
    assert!(metadata.contains("es2:"));
    assert!(!metadata.contains("es1:"));
}

#[test]
fn first_use_app_add_rolls_back_installation_when_target_addition_fails() {
    let project = project("multi", &["dev", "prod"]);

    failure(
        &project,
        &[
            "--environment",
            "missing",
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
            "--yes",
        ],
    );

    assert!(
        !project
            .path()
            .join(".taku/applications/elasticsearch")
            .exists()
    );
}

#[test]
fn first_use_app_add_validates_single_environment_scope_before_installing() {
    let project = project("multi", &["dev", "prod"]);

    failure(
        &project,
        &[
            "--all-environments",
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
            "--yes",
        ],
    );

    assert!(
        !project
            .path()
            .join(".taku/applications/elasticsearch")
            .exists()
    );
}

#[test]
fn local_list_uses_payload_identity_and_exact_selectors() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es1",
            "--url",
            "http://127.0.0.1:9200",
        ],
    );
    let resources = project.path().join("es1/ingest_pipelines");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("human-name.json5"),
        "{ id: 'opaque-123', name: 'Readable', processors: [] }",
    )
    .unwrap();
    std::fs::write(
        project.path().join("unrelated.json"),
        "{\"id\":\"ignored\"}",
    )
    .unwrap();

    let listed = json(
        &project,
        &["list", "--type", "ingest_pipelines", "--id", "opaque-123"],
    );
    assert_eq!(listed["result"][0]["id"], "opaque-123");
    assert_eq!(listed["result"][0]["name"], "Readable");
    assert_eq!(listed["result"].as_array().unwrap().len(), 1);
}

#[test]
fn display_names_may_collide_unless_the_resource_type_declares_them_unique() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let resources = project.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&resources).unwrap();
    for id in ["one", "two"] {
        std::fs::write(
            resources.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","name":"Shared","processors":[]}}"#),
        )
        .unwrap();
    }

    let listed = json(&project, &["list", "--type", "ingest_pipelines"]);
    assert_eq!(listed["result"].as_array().unwrap().len(), 2);
}

#[test]
fn target_sensitive_fields_tighten_the_installed_resource_type() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let project_file = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_file).unwrap()).unwrap();
    config["environments"]["dev"]["targets"]["es"]["sensitive_fields"] =
        serde_yaml::from_str("ingest_pipelines: [/description]\n").unwrap();
    std::fs::write(&project_file, serde_yaml::to_string(&config).unwrap()).unwrap();
    let resources = project.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("Pipeline.json"),
        r#"{"id":"pipe","name":"Pipeline","description":"must not persist"}"#,
    )
    .unwrap();

    let output = failure(&project, &["list"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Sensitive Field /description"));
}

#[test]
fn target_sensitive_fields_cannot_remove_required_identity_or_display_state() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let project_file = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_file).unwrap()).unwrap();
    config["environments"]["dev"]["targets"]["es"]["sensitive_fields"] =
        serde_yaml::from_str("ingest_pipelines: [/id]\n").unwrap();
    std::fs::write(&project_file, serde_yaml::to_string(&config).unwrap()).unwrap();
    let resources = project.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("Pipeline.json"),
        r#"{"id":"pipe","name":"Pipeline"}"#,
    )
    .unwrap();

    let output = failure(&project, &["list"]);

    assert!(String::from_utf8_lossy(&output.stderr).contains("required canonical state"));
}

#[test]
fn target_sensitive_fields_cannot_remove_required_transformation_inputs() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let definition_file = project
        .path()
        .join(".taku/applications/elasticsearch/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_file).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["ingest_pipelines"]["transformations"] =
        serde_yaml::from_str("- { kind: embedded_json, pointer: /payload }\n").unwrap();
    std::fs::write(
        &definition_file,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    let project_file = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_file).unwrap()).unwrap();
    config["environments"]["dev"]["targets"]["es"]["sensitive_fields"] =
        serde_yaml::from_str("ingest_pipelines: [/payload]\n").unwrap();
    std::fs::write(&project_file, serde_yaml::to_string(&config).unwrap()).unwrap();
    let resources = project.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("Pipeline.json"),
        r#"{"id":"pipe","name":"Pipeline","payload":{"x":1}}"#,
    )
    .unwrap();

    let output = failure(&project, &["list"]);

    assert!(String::from_utf8_lossy(&output.stderr).contains("required canonical state"));
}

#[test]
fn sensitive_descendants_inside_an_extracted_document_are_allowed() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let definition_file = project
        .path()
        .join(".taku/applications/elasticsearch/resources.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_file).unwrap()).unwrap();
    definition["target_profile"]["resource_types"]["ingest_pipelines"]["transformations"] =
        serde_yaml::from_str("- { kind: extract, pointer: /payload }\n").unwrap();
    std::fs::write(
        &definition_file,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    let project_file = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_file).unwrap()).unwrap();
    config["environments"]["dev"]["targets"]["es"]["sensitive_fields"] =
        serde_yaml::from_str("ingest_pipelines: [/payload/response_secret]\n").unwrap();
    std::fs::write(&project_file, serde_yaml::to_string(&config).unwrap()).unwrap();
    let resources = project.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("Pipeline.json"),
        r#"{"id":"pipe","name":"Pipeline"}"#,
    )
    .unwrap();

    let listed = json(&project, &["list"]);

    assert_eq!(listed["result"][0]["id"], "pipe");
}

#[test]
fn repeated_environment_selectors_make_cross_environment_scope_explicit() {
    let project = project("multi", &["dev", "prod"]);
    json(&project, &["install", "elasticsearch"]);
    for environment in ["dev", "prod"] {
        json(
            &project,
            &[
                "--environment",
                environment,
                "app",
                "add",
                "elasticsearch",
                "es",
                "--url",
                "http://invalid",
            ],
        );
        let dir = project
            .path()
            .join(format!("{environment}/es/ingest_pipelines"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Pipeline.json"),
            format!("{{\"id\":\"{environment}-pipe\",\"name\":\"Pipeline\",\"processors\":[]}}"),
        )
        .unwrap();
    }
    let listed = json(
        &project,
        &["--environment", "dev", "--environment", "prod", "list"],
    );
    assert_eq!(listed["result"].as_array().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_and_traversal_escaped_resource_inputs() {
    use std::os::unix::fs::symlink;
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "app",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let dir = project.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&dir).unwrap();
    let outside = project.path().join("outside.json");
    std::fs::write(&outside, "{\"id\":\"outside\",\"name\":\"Outside\"}").unwrap();
    symlink(&outside, dir.join("linked.json")).unwrap();
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["list"])
        .assert()
        .failure();
    std::fs::remove_file(dir.join("linked.json")).unwrap();
    std::fs::write(
        dir.join("%2e%2e.json"),
        "{\"id\":\"escaped\",\"name\":\"Escaped\"}",
    )
    .unwrap();
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["list"])
        .assert()
        .failure();
}
