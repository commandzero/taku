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

struct PluginFake {
    url: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl PluginFake {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let installed = Arc::new(AtomicBool::new(false));
        let (captured, ending, state) = (requests.clone(), stop.clone(), installed.clone());
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push((format!("{} {}", request.method(), request.url()), body));
                let plugin = plugin_payload();
                let response = match (request.method(), request.url()) {
                    (&Method::Get, "/api/status") => {
                        Response::from_string(r#"{"version":{"number":"9.4.0"}}"#)
                    }
                    (&Method::Get, "/api/agent_builder/plugins/financial-analysis")
                        if state.load(Ordering::Relaxed) =>
                    {
                        Response::from_string(plugin.to_string())
                    }
                    (&Method::Get, "/api/agent_builder/plugins/financial-analysis") => {
                        Response::from_string("not found").with_status_code(404)
                    }
                    (&Method::Get, "/api/agent_builder/plugins") => {
                        let results = if state.load(Ordering::Relaxed) {
                            vec![plugin]
                        } else {
                            Vec::new()
                        };
                        Response::from_string(json!({"results": results}).to_string())
                    }
                    (&Method::Post, "/api/agent_builder/plugins/install") => {
                        state.store(true, Ordering::Relaxed);
                        Response::from_string(plugin.to_string())
                    }
                    (&Method::Delete, "/api/agent_builder/plugins/financial-analysis") => {
                        state.store(false, Ordering::Relaxed);
                        Response::from_string(r#"{"success":true}"#)
                    }
                    _ => Response::from_string("unexpected request").with_status_code(405),
                };
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for PluginFake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn plugin_payload() -> Value {
    json!({
        "created_at": "2025-01-01T00:00:00.000Z",
        "description": "Financial analysis tools and skills for Claude",
        "id": "financial-analysis",
        "manifest": {
            "author": {"name": "Anthropic", "url": "https://www.anthropic.com"},
            "keywords": ["finance", "analysis"],
            "repository": "https://github.com/anthropics/financial-services-plugins"
        },
        "name": "financial-analysis",
        "skill_ids": ["financial-analysis-analyze-portfolio"],
        "source_url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis",
        "unmanaged_assets": {
            "agents": [],
            "hooks": [],
            "lsp_servers": [],
            "mcp_servers": [],
            "output_styles": []
        },
        "updated_at": "2025-01-01T00:00:00.000Z",
        "version": "1.0.0"
    })
}

#[test]
fn agent_builder_plugins_install_from_source_and_delete_without_force() {
    let fake = PluginFake::start();
    let project = setup(&fake.url);

    let directory = write_plugin_source(&project);

    let selector = [
        "kb",
        "plugins",
        "--namespace",
        "default",
        "financial-analysis",
    ];
    let mut fetch = vec!["fetch"];
    fetch.extend(selector);
    run(&project, &fetch);

    let mut push = vec!["push"];
    push.extend(selector);
    push.extend(["--uncommitted", "allow", "--untracked", "allow"]);
    let installed = run(&project, &push);
    assert_eq!(installed["result"][0]["outcome"], "success");

    run(&project, &fetch);
    let mut status = vec!["status"];
    status.extend(selector);
    let status = run(&project, &status);
    assert_eq!(status["result"][0]["state"], "in_sync");

    let listed = run(
        &project,
        &[
            "list",
            "--remote",
            "kb",
            "plugins",
            "--namespace",
            "default",
        ],
    );
    assert_eq!(listed["result"][0]["id"], "financial-analysis");
    assert_eq!(listed["result"][0]["name"], "financial-analysis");

    let mut forget = vec!["forget"];
    forget.extend(selector);
    run(&project, &forget);
    let mut add = vec!["add"];
    add.extend(selector);
    run(&project, &add);
    let canonical: Value = serde_json::from_str(
        &std::fs::read_to_string(directory.join("financial-analysis.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        canonical,
        json!({
            "_taku": {"id": "financial-analysis"},
            "id": "financial-analysis",
            "name": "financial-analysis",
            "source_url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis"
        })
    );

    let mut remove = vec!["remove"];
    remove.extend(selector);
    run(&project, &remove);
    let deleted = run(&project, &push);
    assert_eq!(deleted["result"][0]["outcome"], "deleted");

    assert_plugin_requests(&fake);
}

fn assert_plugin_requests(fake: &PluginFake) {
    let requests = fake.requests.lock().unwrap();
    let install_body = requests
        .iter()
        .find_map(|(request, body)| {
            (request == "POST /api/agent_builder/plugins/install").then_some(body)
        })
        .expect("plugin install request");
    assert_eq!(
        serde_json::from_str::<Value>(install_body).unwrap(),
        json!({
            "plugin_name": "financial-analysis",
            "url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis"
        })
    );
    assert!(
        requests.iter().any(|(request, _)| {
            request == "DELETE /api/agent_builder/plugins/financial-analysis"
        })
    );
    assert!(
        !requests
            .iter()
            .any(|(request, _)| request.contains("force=true"))
    );
}

fn write_plugin_source(project: &TempDir) -> std::path::PathBuf {
    let directory = project.path().join("kb/default/plugins");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("financial-analysis.json"),
        serde_json::to_string_pretty(&json!({
            "id": "financial-analysis",
            "name": "financial-analysis",
            "source_url": "https://github.com/anthropics/financial-services-plugins/tree/main/financial-analysis"
        }))
        .unwrap(),
    )
    .unwrap();
    directory
}
