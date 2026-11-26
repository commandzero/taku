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
    assert_eq!(listed["result"][0]["version"], ">=9.0.0, <10.0.0");
    assert_eq!(listed["result"][0]["definition_version"], "1.0.0");
    assert_eq!(listed["result"][1]["version"], ">=9.0.0, <10.0.0");
    assert_eq!(listed["result"][1]["definition_version"], "1.0.0");
    assert_eq!(listed["result"][0]["installed"], false);

    let installed = json(&project, &["install", "elasticsearch", "kibana"]);
    assert_eq!(installed["result"][0]["source"], "embedded");
    assert_eq!(installed["result"][0]["version"], ">=9.0.0, <10.0.0");
    assert_eq!(installed["result"][0]["definition_version"], "1.0.0");
    assert!(
        project
            .path()
            .join(".taku/applications/elasticsearch/version-9.yaml")
            .is_file()
    );
    assert!(
        project
            .path()
            .join(".taku/applications/kibana/version-9.yaml")
            .is_file()
    );

    let metadata = std::fs::read_to_string(project.path().join(".taku/project.yaml")).unwrap();
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
            "target",
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
            "target",
            "add",
            "elasticsearch",
            "es2",
            "--url",
            "http://127.0.0.1:9201",
        ],
    );
    std::fs::create_dir_all(project.path().join("dev/es1/ingest_pipelines")).unwrap();
    std::fs::write(
        project.path().join("dev/es1/.target.yaml"),
        "schema_version: 1\nmetadata: { track: true }\n",
    )
    .unwrap();
    std::fs::write(
        project
            .path()
            .join("dev/es1/ingest_pipelines/.resource.yaml"),
        "schema_version: 1\nmetadata: { track: false }\n",
    )
    .unwrap();
    let resource = project
        .path()
        .join("dev/es1/ingest_pipelines/pipeline.json");
    let payload = br#"{"_taku":{"id":"stable-id"},"processors":[]}"#;
    std::fs::write(&resource, payload).unwrap();
    json(
        &project,
        &[
            "target",
            "--environment",
            "prod",
            "add",
            "elasticsearch",
            "es1",
            "--url",
            "http://127.0.0.1:9202",
        ],
    );
    let prod_before = json(&project, &["target", "--environment", "prod"]);
    json(&project, &["target", "rename", "es1", "cluster"]);

    let targets = json(&project, &["target"]);
    let names: Vec<_> = targets["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|target| target["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["cluster", "es2"]);
    assert_eq!(
        json(&project, &["target", "--environment", "prod"]),
        prod_before
    );
    assert_eq!(
        std::fs::read(
            project
                .path()
                .join("dev/cluster/ingest_pipelines/pipeline.json")
        )
        .unwrap(),
        payload
    );
    assert!(project.path().join("dev/cluster/.target.yaml").is_file());
    assert!(
        project
            .path()
            .join("dev/cluster/ingest_pipelines/.resource.yaml")
            .is_file()
    );
    assert!(!project.path().join("dev/es1").exists());
}

#[test]
fn first_use_app_add_rolls_back_installation_when_target_addition_fails() {
    let project = project("multi", &["dev", "prod"]);

    failure(
        &project,
        &[
            "target",
            "--environment",
            "missing",
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
            "target",
            "--all-environments",
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
            "target",
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

    let listed = json(&project, &["list", "es1", "ingest_pipelines", "opaque-123"]);
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
            "target",
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

    let listed = json(&project, &["list", "es", "ingest_pipelines"]);
    assert_eq!(listed["result"].as_array().unwrap().len(), 2);
}

#[test]
fn target_sensitive_fields_tighten_the_installed_resource_type() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "target",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let project_file = project.path().join(".taku/project.yaml");
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
            "target",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let project_file = project.path().join(".taku/project.yaml");
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
            "target",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let definition_file = project
        .path()
        .join(".taku/applications/elasticsearch/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_file).unwrap()).unwrap();
    definition["resource_types"]["ingest_pipelines"][0]["transformations"] =
        serde_yaml::from_str("- { kind: embedded_json, pointer: /payload }\n").unwrap();
    std::fs::write(
        &definition_file,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    let project_file = project.path().join(".taku/project.yaml");
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
            "target",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let definition_file = project
        .path()
        .join(".taku/applications/elasticsearch/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_file).unwrap()).unwrap();
    definition["resource_types"]["ingest_pipelines"][0]["transformations"] =
        serde_yaml::from_str("- { kind: extract, pointer: /payload }\n").unwrap();
    std::fs::write(
        &definition_file,
        serde_yaml::to_string(&definition).unwrap(),
    )
    .unwrap();
    let project_file = project.path().join(".taku/project.yaml");
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
                "target",
                "--environment",
                environment,
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
        &["list", "--environment", "dev", "--environment", "prod"],
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
            "target",
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

#[test]
fn target_names_cannot_escape_their_environment_or_use_project_metadata() {
    for layout in ["single", "multi"] {
        let project = project(
            layout,
            if layout == "single" {
                &["dev"]
            } else {
                &["dev", "prod"]
            },
        );
        json(&project, &["install", "elasticsearch"]);
        json(&project, &["context", "set", "dev"]);
        json(
            &project,
            &[
                "target",
                "add",
                "elasticsearch",
                "es",
                "--url",
                "http://invalid",
            ],
        );
        let tree = project
            .path()
            .join(if layout == "single" { "es" } else { "dev/es" });
        std::fs::create_dir_all(tree.join("ingest_pipelines")).unwrap();
        let resource = tree.join("ingest_pipelines/pipeline.json");
        let payload = br#"{"_taku":{"id":"stable-id"},"processors":[]}"#;
        std::fs::write(&resource, payload).unwrap();
        let metadata = project.path().join(".taku/project.yaml");
        let before = std::fs::read(&metadata).unwrap();
        let absolute = project.path().join("escaped").display().to_string();
        for name in [
            "",
            ".",
            "..",
            ".git",
            ".taku",
            "../escaped",
            "a/b",
            "a\\b",
            &absolute,
        ] {
            for args in [
                vec![
                    "target",
                    "add",
                    "elasticsearch",
                    name,
                    "--url",
                    "http://invalid",
                ],
                vec!["target", "rename", "es", name],
            ] {
                let output = Command::cargo_bin("taku")
                    .unwrap()
                    .current_dir(project.path())
                    .args(&args)
                    .output()
                    .unwrap();
                assert_eq!(output.status.code(), Some(2), "{layout}: {args:?}");
                assert_eq!(std::fs::read(&metadata).unwrap(), before, "{args:?}");
                assert_eq!(std::fs::read(&resource).unwrap(), payload, "{args:?}");
            }
        }
    }
}

#[test]
fn target_rename_rejects_symlinked_cache_before_moving_or_deleting_anything() {
    use std::os::unix::fs::symlink;

    let project = project("multi", &["dev", "prod"]);
    json(&project, &["install", "elasticsearch"]);
    json(&project, &["context", "set", "dev"]);
    json(
        &project,
        &[
            "target",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    let resource = project.path().join("dev/es/ingest_pipelines/pipeline.json");
    std::fs::create_dir_all(resource.parent().unwrap()).unwrap();
    let payload = br#"{"_taku":{"id":"stable-id"},"processors":[]}"#;
    std::fs::write(&resource, payload).unwrap();
    let external = tempfile::tempdir().unwrap();
    std::fs::create_dir(external.path().join("es")).unwrap();
    let unrelated = external.path().join("es/keep.txt");
    std::fs::write(&unrelated, "unrelated data").unwrap();
    std::fs::create_dir_all(project.path().join(".taku/cache")).unwrap();
    symlink(external.path(), project.path().join(".taku/cache/dev")).unwrap();
    let metadata = project.path().join(".taku/project.yaml");
    let before = std::fs::read(&metadata).unwrap();

    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["target", "rename", "es", "renamed"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(std::fs::read(&metadata).unwrap(), before);
    assert_eq!(std::fs::read(&resource).unwrap(), payload);
    assert_eq!(
        std::fs::read_to_string(unrelated).unwrap(),
        "unrelated data"
    );
    assert!(!project.path().join("dev/renamed").exists());
}

#[test]
fn target_rename_does_not_replace_an_unmanaged_destination_directory() {
    let project = project("single", &["dev"]);
    json(&project, &["install", "elasticsearch"]);
    json(
        &project,
        &[
            "target",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    std::fs::create_dir(project.path().join("es")).unwrap();
    std::fs::create_dir(project.path().join("unmanaged")).unwrap();
    let metadata = project.path().join(".taku/project.yaml");
    let before = std::fs::read(&metadata).unwrap();

    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["target", "rename", "es", "unmanaged"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(std::fs::read(&metadata).unwrap(), before);
    assert!(project.path().join("es").is_dir());
    assert!(project.path().join("unmanaged").is_dir());
}

#[test]
fn context_preserves_environment_names_that_require_yaml_quoting() {
    let project = project("multi", &["dev #1", "prod: blue"]);
    json(&project, &["install", "elasticsearch"]);
    for environment in ["dev #1", "prod: blue"] {
        json(&project, &["context", "set", environment]);
        json(
            &project,
            &["target", "add", "elasticsearch", "--url", "http://invalid"],
        );
        let targets = json(&project, &["target"]);
        assert_eq!(targets["result"][0]["environment"], environment);
        assert_eq!(targets["result"][0]["name"], "elasticsearch");
    }
}

#[test]
fn target_operations_reject_symlinked_resource_tree_components() {
    use std::os::unix::fs::symlink;

    for alias in ["target", "environment"] {
        let project = project("multi", &["dev", "prod"]);
        json(&project, &["install", "elasticsearch"]);
        json(&project, &["context", "set", "dev"]);
        json(
            &project,
            &[
                "target",
                "add",
                "elasticsearch",
                "es",
                "--url",
                "http://invalid",
            ],
        );
        let external = tempfile::tempdir().unwrap();
        let tree = if alias == "target" {
            std::fs::create_dir(project.path().join("dev")).unwrap();
            symlink(external.path(), project.path().join("dev/es")).unwrap();
            external.path().to_owned()
        } else {
            symlink(external.path(), project.path().join("dev")).unwrap();
            external.path().join("es")
        };
        std::fs::create_dir_all(tree.join("ingest_pipelines")).unwrap();
        let resource = tree.join("ingest_pipelines/pipeline.json");
        let payload = br#"{"_taku":{"id":"stable-id"},"processors":[]}"#;
        std::fs::write(&resource, payload).unwrap();
        let metadata = project.path().join(".taku/project.yaml");
        let before = std::fs::read(&metadata).unwrap();
        for args in [
            vec!["target", "rename", "es", "renamed"],
            vec![
                "target",
                "add",
                "elasticsearch",
                "es",
                "--url",
                "http://invalid",
            ],
        ] {
            let output = Command::cargo_bin("taku")
                .unwrap()
                .current_dir(project.path())
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{alias}: {args:?}");
            assert_eq!(std::fs::read(&metadata).unwrap(), before);
            assert_eq!(std::fs::read(&resource).unwrap(), payload);
            assert!(!tree.parent().unwrap().join("renamed").exists());
        }
    }
}

#[test]
fn first_use_target_add_requires_authorization_and_rolls_back_invalid_names() {
    let project = project("single", &["dev"]);
    let metadata = project.path().join(".taku/project.yaml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
    config["application_source"] =
        serde_yaml::from_str("location: /this-source-does-not-exist-and-must-not-be-refreshed\n")
            .unwrap();
    std::fs::write(&metadata, serde_yaml::to_string(&config).unwrap()).unwrap();
    let before = std::fs::read(&metadata).unwrap();
    for args in [
        vec![
            "--non-interactive",
            "target",
            "add",
            "elasticsearch",
            "--url",
            "http://invalid",
        ],
        vec![
            "--non-interactive",
            "target",
            "add",
            "elasticsearch",
            "../escaped",
            "--url",
            "http://invalid",
            "--yes",
        ],
    ] {
        let output = Command::cargo_bin("taku")
            .unwrap()
            .current_dir(project.path())
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            !project
                .path()
                .join(".taku/applications/elasticsearch")
                .exists()
        );
        assert_eq!(std::fs::read(&metadata).unwrap(), before);
    }

    json(
        &project,
        &[
            "--non-interactive",
            "target",
            "add",
            "elasticsearch",
            "--url",
            "http://invalid",
            "--yes",
        ],
    );
    let targets = json(&project, &["target"]);
    assert_eq!(targets["result"][0]["name"], "elasticsearch");
    assert_eq!(targets["result"][0]["application"], "elasticsearch");
    assert!(
        project
            .path()
            .join(".taku/applications/elasticsearch/application.yaml")
            .is_file()
    );
    assert!(
        !project
            .path()
            .join(".taku/cache/application-source")
            .exists()
    );
}

#[test]
fn multi_target_scope_requires_context_or_an_explicit_environment() {
    let project = project("multi", &["dev", "prod"]);
    json(&project, &["install", "elasticsearch"]);
    let metadata = project.path().join(".taku/project.yaml");
    let before = std::fs::read(&metadata).unwrap();
    failure(
        &project,
        &[
            "target",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    assert_eq!(std::fs::read(&metadata).unwrap(), before);
    json(
        &project,
        &[
            "target",
            "--environment",
            "dev",
            "add",
            "elasticsearch",
            "es",
            "--url",
            "http://invalid",
        ],
    );
    failure(&project, &["target"]);
    json(&project, &["context", "set", "dev"]);
    let context = project.path().join(".taku/context.yaml");
    let context_before = std::fs::read(&context).unwrap();
    failure(&project, &["context", "set", "missing"]);
    assert_eq!(std::fs::read(context).unwrap(), context_before);
    let targets = json(&project, &["target"]);
    assert_eq!(targets["result"][0]["environment"], "dev");
    assert_eq!(targets["result"][0]["name"], "es");
}
