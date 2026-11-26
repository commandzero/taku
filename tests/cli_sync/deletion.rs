use super::{FakeTarget, output, run, setup};

#[test]
fn push_deletes_only_through_a_matching_guarded_marker_and_consumes_it() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    run(&project, &["remove", "es", "ingest_pipelines", "pipe-1"]);
    let result = run(
        &project,
        &["push", "--uncommitted", "allow", "--untracked", "allow"],
    );
    assert_eq!(result["result"][0]["outcome"], "deleted");
    assert!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "DELETE")
    );
    assert!(
        !std::fs::read_dir(project.path().join("es/ingest_pipelines"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .ends_with(".delete.yaml"))
    );
}

#[test]
fn deletion_marker_push_requires_fetch_and_pull_after_a_hint_change() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    run(&project, &["remove", "es", "ingest_pipelines", "pipe-1"]);
    std::fs::write(
        project.path().join("es/.target.yaml"),
        "schema_version: 1\nmetadata: { track: true }\n",
    )
    .unwrap();

    run(&project, &["fetch"]);
    target.requests.lock().unwrap().clear();
    let blocked = output(
        &project,
        &["push", "--uncommitted", "allow", "--untracked", "allow"],
    );
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("requires Pull"));
    assert!(target.requests.lock().unwrap().is_empty());

    run(&project, &["pull", "--yes"]);
    run(
        &project,
        &["push", "--uncommitted", "allow", "--untracked", "allow"],
    );
    assert!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "DELETE")
    );
}

#[test]
fn pull_reconciles_a_deletion_guard_when_metadata_tracking_is_disabled() {
    let target = FakeTarget::start();
    let project = setup(&target);
    let hint = project.path().join("es/.target.yaml");
    std::fs::write(&hint, "schema_version: 1\nmetadata: { track: true }\n").unwrap();
    run(&project, &["fetch"]);
    run(&project, &["remove", "es", "ingest_pipelines", "pipe-1"]);
    let marker = std::fs::read_dir(project.path().join("es/ingest_pipelines"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".delete.yaml")
        })
        .unwrap()
        .path();
    let tracked_guard = std::fs::read_to_string(&marker).unwrap();
    std::fs::write(&hint, "schema_version: 1\nmetadata: { track: false }\n").unwrap();

    run(&project, &["fetch"]);
    let blocked = output(
        &project,
        &["push", "--uncommitted", "allow", "--untracked", "allow"],
    );
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("requires Pull"));

    let pulled = run(&project, &["pull", "--yes"]);
    assert_eq!(pulled["result"][0]["outcome"], "pulled");
    assert_ne!(std::fs::read_to_string(&marker).unwrap(), tracked_guard);
    run(
        &project,
        &["push", "--uncommitted", "allow", "--untracked", "allow"],
    );
    assert!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "DELETE")
    );
}

#[test]
fn fetch_explicitly_reobserves_resources_marked_for_deletion() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    run(&project, &["remove", "es", "ingest_pipelines", "pipe-1"]);
    target.requests.lock().unwrap().clear();

    let fetched = run(&project, &["fetch"]);

    assert_eq!(fetched["result"][0]["id"], "pipe-1");
    assert!(
        target
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.method == "GET" && request.path == "/_ingest/pipeline/pipe-1")
    );
}

#[test]
fn deletion_markers_reject_unknown_schema_versions() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(&project, &["fetch"]);
    run(&project, &["remove", "es", "ingest_pipelines", "pipe-1"]);
    let marker = std::fs::read_dir(project.path().join("es/ingest_pipelines"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".delete.yaml")
        })
        .unwrap()
        .path();
    let text = std::fs::read_to_string(&marker).unwrap();
    std::fs::write(
        &marker,
        text.replacen("schema_version: 1", "schema_version: 99", 1),
    )
    .unwrap();

    let failed = output(&project, &["forget", "es", "ingest_pipelines", "pipe-1"]);

    assert!(!failed.status.success());
    assert!(marker.exists());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("schema version"));
}

#[test]
fn deletion_marker_binding_must_match_the_tree_that_contains_it() {
    let target = FakeTarget::start();
    let project = setup(&target);
    run(
        &project,
        &[
            "target",
            "add",
            "elasticsearch",
            "other",
            "--url",
            &target.url,
        ],
    );
    run(&project, &["fetch"]);
    run(&project, &["remove", "es", "ingest_pipelines", "pipe-1"]);
    let marker = std::fs::read_dir(project.path().join("es/ingest_pipelines"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".delete.yaml")
        })
        .unwrap()
        .path();
    let text = std::fs::read_to_string(&marker).unwrap();
    std::fs::write(&marker, text.replacen("target: es", "target: other", 1)).unwrap();
    let requests = target.requests.lock().unwrap().len();

    let failed = output(
        &project,
        &["push", "--untracked", "allow", "--uncommitted", "allow"],
    );

    assert!(!failed.status.success());
    assert_eq!(target.requests.lock().unwrap().len(), requests);
    assert!(String::from_utf8_lossy(&failed.stderr).contains("binding"));
}

#[cfg(unix)]
#[test]
fn deletion_marker_discovery_rejects_symlinked_inputs() {
    use std::os::unix::fs::symlink;
    let target = FakeTarget::start();
    let project = setup(&target);
    let outside = project.path().join("outside.delete.yaml");
    std::fs::write(
        &outside,
        "schema_version: 1\nenvironment: dev\ntarget: es\ntype: ingest_pipelines\nid: pipe-1\nguard: guard\nsource_path: ignored\n",
    )
    .unwrap();
    let linked = project
        .path()
        .join("es/ingest_pipelines/Linked.delete.yaml");
    symlink(&outside, &linked).unwrap();

    let failed = output(&project, &["forget", "es", "ingest_pipelines", "pipe-1"]);

    assert!(!failed.status.success());
    assert!(linked.exists());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("symlinked"));
}
