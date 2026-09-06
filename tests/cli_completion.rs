use assert_cmd::Command;
use resource_control::{CompletionIntent, CompletionQuery, completion_candidates, list_targets};
use serde_json::Value;
use std::collections::BTreeMap;
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

fn project() -> TempDir {
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
    run(&project, &["install", "elasticsearch", "kibana"]);
    run(
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
    run(
        &project,
        &[
            "target",
            "--environment",
            "dev",
            "add",
            "kibana",
            "kb",
            "--url",
            "http://invalid",
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
            "prod-es",
            "--url",
            "http://invalid",
        ],
    );
    let directory = project.path().join("dev/es/ingest_pipelines");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("Pipeline.json"),
        r#"{"id":"pipe-1","name":"Pipeline","processors":[]}"#,
    )
    .unwrap();
    let namespaced = project.path().join("dev/kb/team/saved_objects");
    std::fs::create_dir_all(&namespaced).unwrap();
    std::fs::write(
        namespaced.join("Dashboard.json"),
        r#"{"id":"dashboard-1","type":"dashboard","attributes":{"title":"Dashboard"}}"#,
    )
    .unwrap();
    let metadata = project.path().join(".taku/project.yaml");
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&metadata).unwrap()).unwrap();
    value["environments"]["dev"]["provider"] = serde_yaml::from_str(
        "fields:\n  authorization: TOKEN\n  cloud_id: CLOUD_ID\noptional: true\n",
    )
    .unwrap();
    std::fs::write(metadata, serde_yaml::to_string(&value).unwrap()).unwrap();
    project
}

fn query(project: &TempDir, intent: CompletionIntent) -> CompletionQuery {
    let mut query = CompletionQuery::new(project.path(), intent);
    query.environment = Some("dev".into());
    query
}

#[test]
fn candidate_interface_filters_sorts_describes_and_excludes_values() {
    let project = project();

    let mut environments = query(&project, CompletionIntent::Environment);
    environments.prefix = "p".into();
    assert_eq!(
        completion_candidates(&environments).unwrap()[0].value,
        "prod"
    );

    let targets = completion_candidates(&query(&project, CompletionIntent::Target)).unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|candidate| candidate.value.as_str())
            .collect::<Vec<_>>(),
        ["es", "kb"]
    );
    assert!(
        targets
            .iter()
            .all(|candidate| candidate.description.is_some())
    );

    let mut types = query(
        &project,
        CompletionIntent::LocalResourceType {
            include_markers: true,
        },
    );
    types.target = Some("es".into());
    let types = completion_candidates(&types).unwrap();
    assert!(
        types
            .iter()
            .any(|candidate| candidate.value == "ingest_pipelines")
    );

    let mut ids = query(
        &project,
        CompletionIntent::LocalResourceId {
            include_markers: true,
        },
    );
    ids.target = Some("es".into());
    ids.resource_type = Some("ingest_pipelines".into());
    assert_eq!(completion_candidates(&ids).unwrap()[0].value, "pipe-1");
    ids.selected.push("pipe-1".into());
    assert!(completion_candidates(&ids).unwrap().is_empty());

    std::fs::remove_file(project.path().join("dev/es/ingest_pipelines/Pipeline.json")).unwrap();
    std::fs::write(
        project
            .path()
            .join("dev/es/ingest_pipelines/Pipeline.delete.yaml"),
        "schema_version: 1\nenvironment: dev\ntarget: es\ntype: ingest_pipelines\nid: pipe-1\nguard: test\nsource_path: dev/es/ingest_pipelines/Pipeline.json\n",
    )
    .unwrap();
    ids.selected.clear();
    assert_eq!(completion_candidates(&ids).unwrap()[0].value, "pipe-1");
    let mut resources_only = query(
        &project,
        CompletionIntent::LocalResourceId {
            include_markers: false,
        },
    );
    resources_only.target = Some("es".into());
    resources_only.resource_type = Some("ingest_pipelines".into());
    assert!(completion_candidates(&resources_only).unwrap().is_empty());

    let provider = completion_candidates(&query(&project, CompletionIntent::ProviderKey)).unwrap();
    assert_eq!(
        provider
            .iter()
            .map(|candidate| candidate.value.as_str())
            .collect::<Vec<_>>(),
        ["authorization=", "cloud_id="]
    );

    let mut namespaces = query(&project, CompletionIntent::Namespace { remote: false });
    namespaces.target = Some("kb".into());
    namespaces.resource_type = Some("saved_objects".into());
    assert!(
        completion_candidates(&namespaces)
            .unwrap()
            .iter()
            .map(|candidate| candidate.value.as_str())
            .eq(["default", "team"])
    );

    let mut non_namespaced = query(&project, CompletionIntent::Namespace { remote: false });
    non_namespaced.target = Some("es".into());
    non_namespaced.resource_type = Some("ingest_pipelines".into());
    assert!(completion_candidates(&non_namespaced).unwrap().is_empty());

    let mut namespaced_ids = query(
        &project,
        CompletionIntent::LocalResourceId {
            include_markers: false,
        },
    );
    namespaced_ids.target = Some("kb".into());
    namespaced_ids.resource_type = Some("saved_objects".into());
    assert!(completion_candidates(&namespaced_ids).unwrap().is_empty());
    namespaced_ids.namespace = Some("team".into());
    assert_eq!(
        completion_candidates(&namespaced_ids).unwrap()[0].value,
        "dashboard-1"
    );
}

#[test]
fn application_and_target_candidates_follow_command_intent() {
    let project = project();
    let target_output = run(&project, &["target", "--environment", "dev"]);
    assert_eq!(target_output["result"][0]["name"], "es");
    assert_eq!(target_output["result"][0]["application"], "elasticsearch");
    let updates =
        completion_candidates(&query(&project, CompletionIntent::UpdateApplication)).unwrap();
    assert_eq!(
        updates
            .iter()
            .map(|candidate| candidate.value.as_str())
            .collect::<Vec<_>>(),
        ["elasticsearch", "kibana"]
    );
    assert!(
        completion_candidates(&query(&project, CompletionIntent::InstallApplication))
            .unwrap()
            .is_empty()
    );
    let mut remaining = query(&project, CompletionIntent::UpdateApplication);
    remaining.selected.push("elasticsearch".into());
    assert_eq!(
        completion_candidates(&remaining).unwrap()[0].value,
        "kibana"
    );
    let listed = list_targets(project.path(), Some("prod")).unwrap();
    assert_eq!(listed[0].name, "prod-es");
    assert_eq!(listed[0].application, "elasticsearch");
    let mut promotion = query(&project, CompletionIntent::PromotionTarget);
    promotion.environment = Some("prod".into());
    assert_eq!(
        completion_candidates(&promotion).unwrap()[0].value,
        "prod-es"
    );
}

#[test]
fn completion_candidate_failures_are_read_only() {
    let project = project();
    let before = project_files(&project);
    let mut invalid = query(&project, CompletionIntent::RemoteResourceType);
    invalid.target = Some("missing".into());
    assert!(completion_candidates(&invalid).is_err());
    assert_eq!(project_files(&project), before);
}

#[test]
fn all_supported_shell_scripts_are_raw_and_sourceable() {
    let project = project();
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let output = Command::cargo_bin("taku")
            .unwrap()
            .current_dir(project.path())
            .args(["completion", shell])
            .output()
            .unwrap();
        assert!(output.status.success(), "{shell}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains("taku"), "{shell}");
        assert!(!script.contains("schema_version"), "{shell}");
    }
}

#[test]
fn command_help_advertises_only_relevant_scope_and_policy_options() {
    let help = |args: &[&str]| {
        let output = Command::cargo_bin("taku")
            .unwrap()
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let validate = help(&["validate", "--help"]);
    for irrelevant in [
        "--environment",
        "--namespace",
        "--set",
        "--target",
        "--type",
        "--id",
    ] {
        assert!(!validate.contains(irrelevant), "{irrelevant}");
    }
    let status = help(&["status", "--help"]);
    assert!(status.contains("--environment"));
    assert!(status.contains("--namespace"));
    assert!(!status.contains("--set"));
    let fetch = help(&["fetch", "--help"]);
    assert!(fetch.contains("--set"));
    let app = help(&["app", "--help"]);
    assert!(app.contains("refresh"));
    assert!(!app.contains("add"));
    assert!(!app.contains("rename"));
    let target = help(&["target", "--help"]);
    assert!(target.contains("add"));
    assert!(target.contains("rename"));
}

#[test]
fn root_help_groups_flat_commands_by_scope() {
    let output = Command::cargo_bin("taku")
        .unwrap()
        .arg("help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();

    let management_heading = help
        .find("taku configuration:")
        .expect("taku configuration help heading");
    let resource_heading = help
        .find("Resource management:")
        .expect("resource help heading");
    assert!(management_heading < resource_heading);

    let management = &help[management_heading..resource_heading];
    for command in [
        "init",
        "app",
        "target",
        "install",
        "update",
        "context",
        "validate",
        "completion",
        "help",
    ] {
        assert!(management.contains(&format!("  {command}")), "{command}");
    }

    let resources = &help[resource_heading..];
    for command in [
        "list", "add", "remove", "forget", "promote", "fetch", "status", "diff", "pull", "push",
    ] {
        assert!(resources.contains(&format!("  {command}")), "{command}");
    }

    Command::cargo_bin("taku")
        .unwrap()
        .args(["add", "es", "ingest_pipelines", "pipe-1", "--help"])
        .assert()
        .success();
}

#[test]
fn runtime_adapter_combines_static_parser_context_with_dynamic_candidates() {
    let project = project();
    let binary = assert_cmd::cargo::cargo_bin!("taku");
    let complete = |words: &[&str]| {
        let output = StdCommand::new(binary)
            .current_dir(project.path())
            .env("COMPLETE", "fish")
            .arg("--")
            .arg("taku")
            .arg("--project")
            .arg(project.path())
            .args(words)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    assert!(complete(&["context", "set", "p"]).contains("prod"));
    assert!(complete(&["update", "e"]).contains("elasticsearch"));
    assert!(complete(&["target", "--environment", "dev", "rename", "e"]).contains("es"));
    assert!(complete(&["status", "--environment", "dev", "e"]).contains("es"));
    let scoped_options = complete(&["status", "--environment", "dev", "es", ""]);
    assert!(!scoped_options.contains("--environment"));
    assert!(!scoped_options.contains("--all-environments"));
    assert!(complete(&["status", "--environment", "dev", "es", "i"]).contains("ingest_pipelines"));
    assert!(
        complete(&[
            "status",
            "--environment",
            "dev",
            "es",
            "ingest_pipelines",
            "p",
        ])
        .contains("pipe-1")
    );
    assert!(
        complete(&[
            "status",
            "--environment",
            "dev",
            "kb",
            "saved_objects",
            "--namespace",
            "t",
        ])
        .contains("team")
    );
    assert!(
        complete(&["fetch", "--environment", "dev", "es", "--set", "a"]).contains("authorization=")
    );
    assert!(
        complete(&[
            "push",
            "--environment",
            "dev",
            "--untracked",
            "allow",
            "es",
            "i",
        ])
        .contains("ingest_pipelines")
    );
    assert!(complete(&["promote", "--from", "prod", "--from-target", "p"]).contains("prod-es"));

    let failed_context = StdCommand::new(binary)
        .current_dir(project.path())
        .env("COMPLETE", "fish")
        .args(["--", "taku", "--project", "missing-project", "status", "e"])
        .output()
        .unwrap();
    assert!(failed_context.status.success());
    assert!(failed_context.stdout.is_empty());
    assert!(failed_context.stderr.is_empty());
}

fn runtime_complete(project: &TempDir, words: &[&str]) -> String {
    let output = StdCommand::new(assert_cmd::cargo::cargo_bin!("taku"))
        .current_dir(project.path())
        .env("COMPLETE", "fish")
        .args(["--", "taku", "--project"])
        .arg(project.path())
        .args(words)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn completion_preserves_global_options_before_nested_commands() {
    let project = project();
    for options in [
        vec!["--output", "json"],
        vec!["--output=json"],
        vec!["--non-interactive"],
    ] {
        let mut words = vec!["target"];
        words.extend(options);
        words.extend(["add", "e"]);
        assert!(
            runtime_complete(&project, &words).contains("elasticsearch"),
            "{words:?}"
        );
    }
}

#[test]
fn completion_preserves_resource_paths_after_end_of_options() {
    let project = project();
    assert!(
        runtime_complete(
            &project,
            &[
                "status",
                "--environment",
                "dev",
                "--",
                "es",
                "ingest_pipelines",
                "p"
            ]
        )
        .contains("pipe-1")
    );
}

#[test]
fn completion_resolves_each_promotion_project() {
    let source = project();
    let destination = project();
    run(&source, &["context", "set", "dev"]);
    run(&destination, &["context", "set", "prod"]);
    let from = format!("--from-project={}", source.path().display());
    let to = format!("--to-project={}", destination.path().display());
    for invoker in [&source, &destination] {
        for projects in [
            vec![
                "--from-project",
                source.path().to_str().unwrap(),
                "--to-project",
                destination.path().to_str().unwrap(),
            ],
            vec![from.as_str(), to.as_str()],
        ] {
            for (flag, expected, excluded) in [
                ("--from-target", "es", "prod-es"),
                ("--to-target", "prod-es", "es"),
            ] {
                let mut words = vec!["promote"];
                words.extend(projects.iter().copied());
                words.extend([flag, ""]);
                let output = runtime_complete(invoker, &words);
                let values: Vec<_> = output
                    .lines()
                    .map(|line| line.split('\t').next().unwrap())
                    .collect();
                assert!(values.contains(&expected), "{flag}: {output}");
                assert!(!values.contains(&excluded), "{flag}: {output}");
            }
        }
    }
    assert!(
        runtime_complete(&destination, &["promote", &from, "--to-target", "p"]).contains("prod-es")
    );
}

#[test]
fn namespace_rules_are_enforced_through_local_commands() {
    let project = project();
    let before = project_files(&project);
    for command in ["list", "status", "diff", "remove", "forget"] {
        for path in [
            vec!["kb", "saved_objects", "dashboard-1"],
            vec!["es", "ingest_pipelines", "pipe-1", "--namespace", "default"],
        ] {
            let output = Command::cargo_bin("taku")
                .unwrap()
                .current_dir(project.path())
                .args([command, "--environment", "dev"])
                .args(&path)
                .output()
                .unwrap();
            assert!(!output.status.success(), "{command} {path:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("--namespace"),
                "{command}: {:?}",
                output.stderr
            );
        }
    }
    let listed = run(
        &project,
        &["list", "--environment", "dev", "kb", "saved_objects"],
    );
    assert_eq!(listed["result"].as_array().unwrap().len(), 1);
    assert_eq!(project_files(&project), before);
}

fn project_files(project: &TempDir) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &std::path::Path, path: &std::path::Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(project.path(), project.path(), &mut out);
    out
}
