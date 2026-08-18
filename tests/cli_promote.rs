use assert_cmd::Command;
use serde_json::{Value, json};
use std::process::Command as StdCommand;
use tempfile::TempDir;

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

fn init_single(name: &str) -> TempDir {
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
        &["init", "--layout", "single", "--environment", name],
    );
    run(&project, &["install", "elasticsearch"]);
    run(
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
    project
}

#[test]
fn promotes_complete_universal_resources_through_destination_owned_mapping() {
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
        &["init", "--layout", "multi", "--environments", "dev,prod"],
    );
    run(&project, &["install", "elasticsearch"]);
    run(
        &project,
        &[
            "target",
            "--environment",
            "dev",
            "add",
            "elasticsearch",
            "source",
            "--url",
            "http://dev.invalid",
        ],
    );
    run(
        &project,
        &[
            "target",
            "--environment",
            "prod",
            "add",
            "elasticsearch",
            "destination",
            "--url",
            "http://prod.invalid",
        ],
    );
    let project_path = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&project_path).unwrap()).unwrap();
    config["environments"]["prod"]["from"] = serde_yaml::Value::String("dev".into());
    config["environments"]["prod"]["targets"]["destination"]["from"] =
        serde_yaml::Value::String("source".into());
    std::fs::write(&project_path, serde_yaml::to_string(&config).unwrap()).unwrap();
    run(&project, &["context", "set", "prod"]);
    let source = project.path().join("dev/source/ingest_pipelines");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("Pipeline.json"),
        serde_json::to_string_pretty(&json!({"id":"pipe-1","name":"Pipeline","processors":[]}))
            .unwrap(),
    )
    .unwrap();
    let result = run(&project, &["promote"]);
    assert_eq!(result["result"][0]["outcome"], "promoted");
    let destination = project
        .path()
        .join("prod/destination/ingest_pipelines/Pipeline.json");
    assert!(destination.is_file());
    assert_eq!(
        serde_json::from_str::<Value>(&std::fs::read_to_string(destination).unwrap()).unwrap()["id"],
        "pipe-1"
    );
}

#[test]
fn cross_project_promotion_requires_exact_installed_application_compatibility() {
    let source = init_single("dev");
    let destination = init_single("prod");
    let resources = source.path().join("es/ingest_pipelines");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(
        resources.join("Pipeline.json"),
        r#"{"id":"pipe-1","name":"Pipeline","processors":[]}"#,
    )
    .unwrap();
    let installed = destination
        .path()
        .join(".taku/applications/elasticsearch/version-9.yml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&installed).unwrap()).unwrap();
    definition["version"] = serde_yaml::Value::String("different".into());
    std::fs::write(&installed, serde_yaml::to_string(&definition).unwrap()).unwrap();

    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(destination.path())
        .args([
            "promote",
            "--from-project",
            source.path().to_str().unwrap(),
            "--from-target",
            "es",
            "--to-target",
            "es",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not exactly compatible"));
    assert!(
        !destination
            .path()
            .join("es/ingest_pipelines/Pipeline.json")
            .exists()
    );
}
