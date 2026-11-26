use assert_cmd::Command;
use serde_json::Value;
use std::process::Command as StdCommand;
use tempfile::TempDir;

#[path = "cli_kibana/mutations.rs"]
mod mutations;
#[path = "cli_kibana/plugins.rs"]
mod plugins;
#[path = "cli_kibana/saved_objects.rs"]
mod saved_objects;
#[path = "cli_kibana/skills.rs"]
mod skills;

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

fn setup(url: &str) -> TempDir {
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
        &["init", "--layout", "single", "--environment", "dev"],
    );
    run(&project, &["install", "kibana"]);
    run(&project, &["target", "add", "kibana", "kb", "--url", url]);
    project
}

fn edit_catalog(project: &TempDir, edit: impl FnOnce(&mut serde_yaml::Value)) {
    let path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut definition);
    std::fs::write(path, serde_yaml::to_string(&definition).unwrap()).unwrap();
}

#[test]
fn local_json5_accepts_comments_and_triple_quoted_human_text() {
    let project = setup("http://invalid");
    let dir = project.path().join("kb/spaces");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Space.json5"),"// deprecated API\n{ id: 'space-1', name: 'Space', description: \"\"\"line one\nline two\"\"\" }").unwrap();
    let listed = run(&project, &["list", "kb", "spaces"]);
    assert_eq!(listed["result"][0]["id"], "space-1");
}
