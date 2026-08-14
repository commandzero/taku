use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn git_project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        StdCommand::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    dir
}

#[test]
fn initializes_single_project_at_exact_git_root_with_yaml_output() {
    let project = git_project();
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["init", "--layout", "single", "--environment", "dev"])
        .assert()
        .success()
        .stdout(predicate::str::contains("schema_version: 1"))
        .stdout(predicate::str::contains("command: init"));

    let metadata = std::fs::read_to_string(project.path().join(".taku/project.yml")).unwrap();
    assert!(metadata.contains("layout: single"));
    assert!(metadata.contains("dev:"));
}

#[test]
fn initializes_multi_project_with_json_output() {
    let project = git_project();
    let output = Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args([
            "--output",
            "json",
            "init",
            "--layout",
            "multi",
            "--environments",
            "dev,prod",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["command"], "init");
    assert_eq!(
        value["result"]["environments"],
        serde_json::json!(["dev", "prod"])
    );
}

#[test]
fn rejects_subdirectory_and_non_worktree() {
    let project = git_project();
    std::fs::create_dir(project.path().join("nested")).unwrap();
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path().join("nested"))
        .args(["init", "--layout", "single", "--environment", "dev"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("exact Git worktree root"));

    let outside = tempfile::tempdir().unwrap();
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(outside.path())
        .args(["init", "--layout", "single", "--environment", "dev"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a Git worktree"));
}

#[test]
fn non_interactive_init_requires_layout_and_environment() {
    let project = git_project();
    Command::cargo_bin("taku")
        .unwrap()
        .current_dir(project.path())
        .args(["--non-interactive", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--layout"));
}
