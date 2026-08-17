use assert_cmd::Command;
use serde_json::Value;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn git_init(path: &std::path::Path) {
    assert!(
        StdCommand::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        StdCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        StdCommand::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
}
fn commit(path: &std::path::Path, message: &str) {
    assert!(
        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        StdCommand::new("git")
            .args(["commit", "-qm", message])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
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

fn run_failure(project: &TempDir, args: &[&str]) -> std::process::Output {
    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.code().is_some_and(|code| code != 0));
    output
}
fn definition(name: &str, definition_version: &str) -> String {
    format!(
        r#"schema_version: 1
version: "{definition_version}"
application: {{ name: {name}, version: "test-application" }}
target_profile:
  headers: {{}}
  fact_probes: []
  resource_types:
    widgets:
      id: {{ pointer: /id, scope: universal }}
      display_name: {{ pointer: /name, strategy: name }}
      operations:
        read: {{ method: GET, path: "/widgets/{{id}}", cardinality: one }}
        upsert: {{ method: PUT, path: "/widgets/{{id}}", cardinality: one }}
"#
    )
}

#[test]
fn refresh_install_and_update_use_an_explicit_git_source_without_implicit_refresh() {
    let source = tempfile::tempdir().unwrap();
    git_init(source.path());
    let app = source.path().join("applications/custom");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("resources.yml"), definition("custom", "1.0.0")).unwrap();
    commit(source.path(), "v1");
    let project = tempfile::tempdir().unwrap();
    git_init(project.path());
    run(
        &project,
        &["init", "--layout", "single", "--environment", "dev"],
    );
    let path = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["application_source"] =
        serde_yaml::from_str(&format!("location: {}\n", source.path().display())).unwrap();
    std::fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
    run(&project, &["app", "refresh"]);
    let listed = run(&project, &["app"]);
    assert!(
        listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "custom")
    );
    let installed = run(&project, &["install", "custom"]);
    assert_eq!(installed["result"][0]["source"], "git");
    let installed_path = project
        .path()
        .join(".taku/applications/custom/resources.yml");
    assert!(
        std::fs::read_to_string(&installed_path)
            .unwrap()
            .contains("revision:")
    );
    std::fs::write(app.join("resources.yml"), definition("custom", "2.0.0")).unwrap();
    commit(source.path(), "v2");
    let before = std::fs::read_to_string(&installed_path).unwrap();
    let listed = run(&project, &["app"]);
    let custom = listed["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "custom")
        .unwrap();
    assert_eq!(custom["version"], "test-application");
    assert_eq!(custom["definition_version"], "1.0.0");
    assert_eq!(std::fs::read_to_string(&installed_path).unwrap(), before);
    run(&project, &["app", "refresh"]);
    run(&project, &["update", "custom"]);
    assert!(
        std::fs::read_to_string(&installed_path)
            .unwrap()
            .contains("2.0.0")
    );
}

#[test]
fn multi_application_update_replaces_none_when_staging_one_candidate_fails() {
    let source = tempfile::tempdir().unwrap();
    git_init(source.path());
    for name in ["alpha", "beta"] {
        let app = source.path().join("applications").join(name);
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("resources.yml"), definition(name, "1.0.0")).unwrap();
    }
    commit(source.path(), "v1");

    let project = tempfile::tempdir().unwrap();
    git_init(project.path());
    run(
        &project,
        &["init", "--layout", "single", "--environment", "dev"],
    );
    let path = project.path().join(".taku/project.yml");
    let mut config: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["application_source"] =
        serde_yaml::from_str(&format!("location: {}\n", source.path().display())).unwrap();
    std::fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
    run(&project, &["app", "refresh"]);
    run(&project, &["install", "alpha", "beta"]);

    for name in ["alpha", "beta"] {
        std::fs::write(
            source
                .path()
                .join("applications")
                .join(name)
                .join("resources.yml"),
            definition(name, "2.0.0"),
        )
        .unwrap();
    }
    commit(source.path(), "v2");
    run(&project, &["app", "refresh"]);
    let alpha = project
        .path()
        .join(".taku/applications/alpha/resources.yml");
    let before = std::fs::read_to_string(&alpha).unwrap();
    std::fs::create_dir(
        project
            .path()
            .join(".taku/applications/beta/resources.next"),
    )
    .unwrap();

    run_failure(&project, &["update"]);

    assert_eq!(std::fs::read_to_string(alpha).unwrap(), before);
}

#[test]
fn update_without_from_preserves_each_installed_git_source() {
    let source_a = tempfile::tempdir().unwrap();
    git_init(source_a.path());
    let app_a = source_a.path().join("applications/custom");
    std::fs::create_dir_all(&app_a).unwrap();
    std::fs::write(app_a.join("resources.yml"), definition("custom", "1.0.0")).unwrap();
    commit(source_a.path(), "source-a-v1");

    let source_b = tempfile::tempdir().unwrap();
    git_init(source_b.path());
    let app_b = source_b.path().join("applications/custom");
    std::fs::create_dir_all(&app_b).unwrap();
    std::fs::write(app_b.join("resources.yml"), definition("custom", "9.0.0")).unwrap();
    commit(source_b.path(), "source-b-v9");

    let project = tempfile::tempdir().unwrap();
    git_init(project.path());
    run(
        &project,
        &["init", "--layout", "single", "--environment", "dev"],
    );
    run(
        &project,
        &[
            "app",
            "refresh",
            "--from",
            source_a.path().to_str().unwrap(),
        ],
    );
    run(&project, &["install", "custom"]);

    std::fs::write(app_a.join("resources.yml"), definition("custom", "2.0.0")).unwrap();
    commit(source_a.path(), "source-a-v2");
    run(
        &project,
        &[
            "app",
            "refresh",
            "--from",
            source_b.path().to_str().unwrap(),
        ],
    );

    run(&project, &["update", "custom"]);

    let installed = std::fs::read_to_string(
        project
            .path()
            .join(".taku/applications/custom/resources.yml"),
    )
    .unwrap();
    assert!(installed.contains("2.0.0"));
    assert!(!installed.contains("9.0.0"));
    assert!(installed.contains(source_a.path().to_str().unwrap()));
}

#[test]
fn invalid_higher_precedence_cache_and_metadata_are_rejected() {
    let source = tempfile::tempdir().unwrap();
    git_init(source.path());
    let app = source.path().join("applications/custom");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("resources.yml"), definition("custom", "1.0.0")).unwrap();
    commit(source.path(), "valid");
    let project = tempfile::tempdir().unwrap();
    git_init(project.path());
    run(
        &project,
        &["init", "--layout", "single", "--environment", "dev"],
    );
    run(
        &project,
        &["app", "refresh", "--from", source.path().to_str().unwrap()],
    );
    let cached = project
        .path()
        .join(".taku/cache/application-source/applications/custom/resources.yml");
    std::fs::write(&cached, "not: a valid application\n").unwrap();
    let listed = run_failure(&project, &["app"]);
    assert!(String::from_utf8_lossy(&listed.stderr).contains("invalid Application definition"));

    std::fs::write(&cached, definition("custom", "1.0.0")).unwrap();
    let metadata = project
        .path()
        .join(".taku/cache/application-source/.source.yml");
    let text = std::fs::read_to_string(&metadata).unwrap();
    std::fs::write(
        &metadata,
        text.replacen("schema_version: 1", "schema_version: 99", 1),
    )
    .unwrap();
    let installed = run_failure(&project, &["install", "custom"]);
    assert!(String::from_utf8_lossy(&installed.stderr).contains("schema version"));
}
