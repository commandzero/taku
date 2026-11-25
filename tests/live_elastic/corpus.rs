use super::{authorization, run_at};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[test]
#[ignore = "requires the ephemeral taku-test-project plus Elasticsearch and Kibana on localhost"]
fn round_trips_the_complete_esdiag_resource_corpus() {
    let authorization = authorization();
    let project = prepare_corpus(&authorization);

    let validated = run_at(&project, &authorization, &["validate"]);
    assert_eq!(validated["result"]["valid"], true);

    let skill_path = check_inventory(&project, &authorization);
    run_at(&project, &authorization, &["fetch", "es"]);
    run_at(&project, &authorization, &["fetch", "kb"]);
    run_at(&project, &authorization, &["pull", "--yes"]);

    check_skill_projection(&project, &skill_path);
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
fn prepare_corpus(authorization: &str) -> PathBuf {
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
            .join("esdiag-assets/elasticsearch/assets.yaml")
            .exists(),
        "the copied ESDiag source corpus is missing"
    );
    let skill_directory = project.join("kb/esdiag/skills/agentic-diagnostic-assistant");
    if !skill_directory.join("SKILL.md").is_file() {
        run_at(
            &project,
            authorization,
            &[
                "add",
                "kb",
                "--namespace",
                "esdiag",
                "skills",
                "agentic-diagnostic-assistant",
            ],
        );
    }
    project
}

fn check_inventory(project: &Path, authorization: &str) -> String {
    let inventory = run_at(project, authorization, &["list"]);
    let resources = inventory["result"].as_array().unwrap();
    assert_eq!(resources.len(), 130);
    assert_eq!(
        resources
            .iter()
            .filter(|resource| resource["type"] == "saved_objects")
            .count(),
        90
    );
    for resource in resources
        .iter()
        .filter(|resource| resource["type"] == "saved_objects")
    {
        let path = project.join(resource["path"].as_str().unwrap());
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str()),
            value["type"].as_str(),
            "Saved Object {} must be stored beneath its type directory",
            path.display()
        );
    }
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
    skill_path.to_owned()
}

fn check_skill_projection(project: &Path, skill_path: &str) {
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
