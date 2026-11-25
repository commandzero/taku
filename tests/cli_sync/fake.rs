use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server};

#[derive(Clone, Debug)]
pub(super) struct Captured {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) authorization: Option<String>,
    pub(super) if_match: Option<String>,
    pub(super) body: String,
}

pub(super) struct FakeTarget {
    pub(super) url: String,
    pub(super) version: Arc<Mutex<String>>,
    pub(super) requests: Arc<Mutex<Vec<Captured>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeTarget {
    pub(super) fn start() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let resource = Arc::new(Mutex::new(
            json!({"id":"pipe-1","name":"Pipeline","description":"remote","processors":[],"version":"7","created_date_millis":1000,"modified_date_millis":2000,"_secret":"NEVER-PERSIST"}),
        ));
        let version = Arc::new(Mutex::new(String::from("9.1.0")));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (state, remote_version, captured, stopping) = (
            resource.clone(),
            version.clone(),
            requests.clone(),
            stop.clone(),
        );
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let authorization = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("authorization"))
                    .map(|h| h.value.as_str().to_owned());
                let if_match = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("if-match"))
                    .map(|header| header.value.as_str().to_owned());
                captured.lock().unwrap().push(Captured {
                    method: request.method().as_str().into(),
                    path: request.url().into(),
                    authorization,
                    if_match,
                    body: body.clone(),
                });
                let response = reply(&state, &remote_version, &request, &body);
                request.respond(response).unwrap();
            }
        });
        Self {
            url,
            version,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FakeTarget {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn reply(
    state: &Mutex<Value>,
    remote_version: &Mutex<String>,
    request: &Request,
    body: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let payload = match (request.method(), request.url()) {
        (&Method::Get, "/_ingest/pipeline/pipe-1") => {
            json!({"pipe-1": state.lock().unwrap().clone()})
        }
        (&Method::Get, "/_ingest/pipeline/pipe-2") => {
            json!({"pipe-2": {"name": "Second", "description": "adopt me", "processors": []}})
        }
        (&Method::Get, "/") => {
            json!({"version": {"number": remote_version.lock().unwrap().clone()}})
        }
        (&Method::Get, "/_ingest/pipeline") => json!({
            "pipe-1": state.lock().unwrap().clone(),
            "pipe-2": {"name": "Second", "description": "adopt me", "processors": []}
        }),
        (&Method::Put, path) if path.starts_with("/_ingest/pipeline/pipe-1") => {
            *state.lock().unwrap() = serde_json::from_str(body).unwrap();
            json!({"acknowledged": true})
        }
        (&Method::Delete, "/_ingest/pipeline/pipe-1") => {
            return Response::from_string("{\"acknowledged\":true}");
        }
        (&Method::Post, "/jobs") => {
            return json_response("{\"id\":\"job-1\",\"name\":\"Pending Job\",\"enabled\":true}");
        }
        (&Method::Post, "/mapped-jobs") => json!({
            "assigned_id": "job-1",
            "resource": {"name": "Pending Job", "enabled": false},
            "acknowledged": true
        }),
        _ => return Response::from_string("not found").with_status_code(404),
    };
    json_response(payload.to_string())
}

fn json_response(body: impl Into<String>) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_header(Header::from_bytes("content-type", "application/json").unwrap())
}
