use super::{run, setup};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tiny_http::{Method, Response, Server};

struct SkillFake {
    url: String,
    content: Arc<Mutex<String>>,
    bodies: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SkillFake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let content = Arc::new(Mutex::new(
            "# Agentic Diagnostic Assistant\n\nUse the referenced runbook.\n".to_owned(),
        ));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let ending = stop.clone();
        let served_content = content.clone();
        let captured_bodies = bodies.clone();
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                if !body.is_empty() {
                    captured_bodies.lock().unwrap().push(body);
                }
                let skill = json!({
                    "id": "agentic-diagnostic-assistant",
                    "name": "Agentic Diagnostic Assistant",
                    "description": "Diagnose Elastic Stack cases",
                    "experimental": true,
                    "metadata": {"owner": "support"},
                    "content": served_content.lock().unwrap().clone(),
                    "referenced_content": [{
                        "name": "runbook",
                        "relativePath": "./references",
                        "content": "# Runbook\n\nInspect diagnostics.\n"
                    }]
                });
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/api/status") => {
                        Response::from_string(r#"{"version":{"number":"9.4.0"}}"#)
                    }
                    (&Method::Get, "/api/agent_builder/skills") => {
                        Response::from_string(json!({"results": [skill]}).to_string())
                    }
                    (&Method::Get, "/api/agent_builder/skills/agentic-diagnostic-assistant") => {
                        Response::from_string(skill.to_string())
                    }
                    (&Method::Put, "/api/agent_builder/skills/agentic-diagnostic-assistant") => {
                        Response::from_string(r#"{"ok":true}"#)
                    }
                    _ => Response::from_string("not found").with_status_code(404),
                };
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            content,
            bodies,
            stop,
            thread: Some(thread),
        }
    }

    fn set_content(&self, content: &str) {
        *self.content.lock().unwrap() = content.to_owned();
    }
}

impl Drop for SkillFake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn configure_skill_projection(project: &TempDir) {
    let definition_path = project
        .path()
        .join(".taku/applications/kibana/version-9.yaml");
    let mut definition: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&definition_path).unwrap()).unwrap();
    let skill = &mut definition["resource_types"]["skills"][0];
    skill["display_name"] = serde_yaml::to_value(json!({
        "pointer": "/name",
        "strategy": "id",
        "unique": true
    }))
    .unwrap();
    skill["filesystem"] = serde_yaml::to_value(json!({
        "split": "frontmatter_markdown",
        "merge": "frontmatter_markdown",
        "frontmatter_markdown": {
            "document": "SKILL.md",
            "body_pointer": "/content",
            "referenced_files": {
                "pointer": "/referenced_content",
                "path_pointer": "/relativePath",
                "name_pointer": "/name",
                "content_pointer": "/content",
                "extension": "md"
            }
        }
    }))
    .unwrap();
    std::fs::write(definition_path, serde_yaml::to_string(&definition).unwrap()).unwrap();
}

#[test]
fn frontmatter_markdown_resources_merge_from_a_directory_tree() {
    let project = setup("http://invalid");

    configure_skill_projection(&project);

    let skill = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    std::fs::create_dir_all(skill.join("references")).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nid: agentic-diagnostic-assistant\nname: Agentic Diagnostic Assistant\ndescription: Diagnose Elastic Stack cases\nexperimental: true\nmetadata:\n  owner: support\n---\n\n# Agentic Diagnostic Assistant\n\nUse the referenced runbook.\n",
    )
    .unwrap();
    std::fs::write(
        skill.join("references/runbook.md"),
        "# Runbook\n\nInspect diagnostics.\n",
    )
    .unwrap();

    let listed = run(
        &project,
        &["list", "kb", "skills", "--namespace", "default"],
    );
    let resource = &listed["result"][0];
    assert_eq!(resource["id"], "agentic-diagnostic-assistant");
    assert_eq!(resource["name"], "Agentic Diagnostic Assistant");
    assert_eq!(
        resource["path"],
        "kb/default/skills/agentic-diagnostic-assistant"
    );
}

#[test]
fn adding_a_frontmatter_markdown_resource_splits_it_to_a_directory_tree() {
    let fake = SkillFake::start();
    let project = setup(&fake.url);
    configure_skill_projection(&project);

    run(
        &project,
        &[
            "add",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );

    let directory = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    let markdown = std::fs::read_to_string(directory.join("SKILL.md")).unwrap();
    assert!(markdown.starts_with("---\n"));
    assert!(!markdown.contains("_taku"));
    assert!(markdown.contains("id: agentic-diagnostic-assistant"));
    assert!(markdown.contains("owner: support"));
    assert!(markdown.ends_with("# Agentic Diagnostic Assistant\n\nUse the referenced runbook.\n"));
    assert_eq!(
        std::fs::read_to_string(directory.join("references/runbook.md")).unwrap(),
        "# Runbook\n\nInspect diagnostics.\n"
    );
    assert!(!directory.with_extension("json").exists());
}

#[test]
fn pulling_a_frontmatter_markdown_resource_replaces_its_directory_projection() {
    let fake = SkillFake::start();
    let project = setup(&fake.url);

    run(
        &project,
        &[
            "add",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );
    let hint = project.path().join("kb/default/skills/.resource.yaml");
    let hint_bytes =
        b"# Keep provenance for this directory.\nschema_version: 1\nmetadata: { track: true }\n";
    std::fs::write(&hint, hint_bytes).unwrap();
    run(
        &project,
        &[
            "fetch",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );
    fake.set_content("# Agentic Diagnostic Assistant\n\nUpdated remotely.\n");
    run(
        &project,
        &[
            "fetch",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );
    let pulled = run(
        &project,
        &[
            "pull",
            "--yes",
            "kb",
            "skills",
            "--namespace",
            "default",
            "agentic-diagnostic-assistant",
        ],
    );

    assert_eq!(pulled["result"][0]["outcome"], "pulled");
    let markdown = std::fs::read_to_string(
        project
            .path()
            .join("kb/default/skills/agentic-diagnostic-assistant/SKILL.md"),
    )
    .unwrap();
    assert_eq!(std::fs::read(&hint).unwrap(), hint_bytes);
    assert!(markdown.ends_with("# Agentic Diagnostic Assistant\n\nUpdated remotely.\n"));
}

#[test]
fn removing_a_projected_resource_replaces_the_directory_with_a_deletion_marker() {
    let fake = SkillFake::start();
    let project = setup(&fake.url);
    let selector = [
        "kb",
        "skills",
        "--namespace",
        "default",
        "agentic-diagnostic-assistant",
    ];
    let mut add = vec!["add"];
    add.extend(selector);
    run(&project, &add);
    let mut fetch = vec!["fetch"];
    fetch.extend(selector);
    run(&project, &fetch);

    let mut remove = vec!["remove"];
    remove.extend(selector);
    run(&project, &remove);

    let directory = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    assert!(!directory.exists());
    assert!(
        project
            .path()
            .join("kb/default/skills/agentic-diagnostic-assistant.delete.yaml")
            .is_file()
    );
}

#[test]
fn pushing_a_projected_resource_merges_passthrough_frontmatter_and_files() {
    let fake = SkillFake::start();
    let project = setup(&fake.url);
    let selector = [
        "kb",
        "skills",
        "--namespace",
        "default",
        "agentic-diagnostic-assistant",
    ];
    let mut add = vec!["add"];
    add.extend(selector);
    run(&project, &add);
    let mut fetch = vec!["fetch"];
    fetch.extend(selector);
    run(&project, &fetch);

    let directory = project
        .path()
        .join("kb/default/skills/agentic-diagnostic-assistant");
    let markdown = std::fs::read_to_string(directory.join("SKILL.md")).unwrap();
    std::fs::write(
        directory.join("SKILL.md"),
        markdown.replace("owner: support", "owner: field-engineering"),
    )
    .unwrap();
    std::fs::write(
        directory.join("references/runbook.md"),
        "# Runbook\n\nInspect the updated diagnostics.\n",
    )
    .unwrap();
    let mut push = vec!["push", "--untracked", "allow", "--uncommitted", "allow"];
    push.extend(selector);
    run(&project, &push);

    let bodies = fake.bodies.lock().unwrap();
    let body: Value = serde_json::from_str(bodies.last().unwrap()).unwrap();
    assert_eq!(body["metadata"]["owner"], "field-engineering");
    assert_eq!(
        body["referenced_content"][0]["content"],
        "# Runbook\n\nInspect the updated diagnostics.\n"
    );
    assert!(
        body["content"]
            .as_str()
            .unwrap()
            .contains("Use the referenced runbook")
    );
}
