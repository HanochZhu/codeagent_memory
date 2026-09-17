use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::tempdir;

fn cam_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_cam").into()
}

fn write_fixture(root: &std::path::Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn run() {
    let _ = add(1, 2);
    helper();
}

fn helper() {}
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("py")).unwrap();
    fs::write(
        root.join("py/mod.py"),
        "def helper():\n    return 1\n\ndef run():\n    return helper()\n",
    )
    .unwrap();
}

#[test]
fn index_ls_read_ref() {
    let dir = tempdir().unwrap();
    write_fixture(dir.path());

    let status = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());

    let out = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("index")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(report["files"].as_u64().unwrap() >= 2);
    assert!(report["nodes"].as_u64().unwrap() >= 6);

    let ls = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["ls", "src/lib.rs"])
        .output()
        .unwrap();
    assert!(ls.status.success());
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&ls.stdout).unwrap();
    let names: Vec<_> = entries
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(names.contains(&"add"));
    assert!(names.contains(&"run"));

    let read = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["read", "src/lib.rs/add"])
        .output()
        .unwrap();
    assert!(read.status.success());
    let body: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
    assert!(body["source"].as_str().unwrap().contains("fn add"));

    let callers = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["ref", "add", "--dir", "in"])
        .output()
        .unwrap();
    assert!(callers.status.success(), "{}", String::from_utf8_lossy(&callers.stderr));
    let refs: serde_json::Value = serde_json::from_slice(&callers.stdout).unwrap();
    let names: Vec<_> = refs["refs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(names.contains(&"run"), "{refs}");

    let callees = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["ref", "src/lib.rs/run", "--dir", "out"])
        .output()
        .unwrap();
    assert!(callees.status.success());
    let refs: serde_json::Value = serde_json::from_slice(&callees.stdout).unwrap();
    let names: Vec<_> = refs["refs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(names.contains(&"add"));
    assert!(names.contains(&"helper"));
}

#[test]
fn sync_updates_changed_added_and_removed_files() {
    let dir = tempdir().unwrap();
    write_fixture(dir.path());

    let status = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());

    let index = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("index")
        .output()
        .unwrap();
    assert!(index.status.success(), "{}", String::from_utf8_lossy(&index.stderr));

    let noop = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("sync")
        .output()
        .unwrap();
    assert!(noop.status.success(), "{}", String::from_utf8_lossy(&noop.stderr));
    let noop_report: serde_json::Value = serde_json::from_slice(&noop.stdout).unwrap();
    assert_eq!(noop_report["files_added"], 0);
    assert_eq!(noop_report["files_modified"], 0);
    assert_eq!(noop_report["files_removed"], 0);

    fs::write(
        dir.path().join("src/lib.rs"),
        r#"
pub fn add(a: i32, b: i32) -> i32 { a + b }
pub fn extra() { let _ = add(1, 2); }
"#,
    )
    .unwrap();
    fs::write(dir.path().join("src/new.rs"), "pub fn fresh() {}\n").unwrap();
    fs::remove_file(dir.path().join("py/mod.py")).unwrap();

    let sync = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("sync")
        .output()
        .unwrap();
    assert!(sync.status.success(), "{}", String::from_utf8_lossy(&sync.stderr));
    let report: serde_json::Value = serde_json::from_slice(&sync.stdout).unwrap();
    assert_eq!(report["files_added"], 1, "{report}");
    assert_eq!(report["files_modified"], 1, "{report}");
    assert_eq!(report["files_removed"], 1, "{report}");

    let ls = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["ls", "src/lib.rs"])
        .output()
        .unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&ls.stdout).unwrap();
    let names: Vec<_> = entries
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(names.contains(&"extra"), "{entries:?}");
    assert!(!names.contains(&"run"), "{entries:?}");

    let ls_new = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["ls", "src/new.rs"])
        .output()
        .unwrap();
    let new_entries: Vec<serde_json::Value> = serde_json::from_slice(&ls_new.stdout).unwrap();
    let new_names: Vec<_> = new_entries
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(new_names.contains(&"fresh"), "{new_entries:?}");
}

#[test]
fn watch_updates_graph_on_file_change() {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use cam::code::{index_project, ls, watch_project};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_fixture(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let (stop_tx, stop_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let watched = project.clone();
    let handle = thread::spawn(move || {
        watch_project(
            &watched,
            Duration::from_millis(120),
            Some(&stop_rx),
            |report| {
                if report.files_modified + report.files_added > 0 {
                    let _ = done_tx.send(());
                }
            },
        )
    });

    thread::sleep(Duration::from_millis(400));
    fs::write(
        dir.path().join("src/lib.rs"),
        r#"
pub fn add(a: i32, b: i32) -> i32 { a + b }
pub fn extra() {}
"#,
    )
    .unwrap();

    let synced = done_rx.recv_timeout(Duration::from_secs(8));
    let _ = stop_tx.send(());
    handle.join().unwrap().expect("watch_project");
    assert!(synced.is_ok(), "watcher did not sync after source change");

    let entries = ls(&project, Some("src/lib.rs")).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"extra"), "{names:?}");
}

#[test]
fn empty_graph_asks_to_index() {
    use cam::code::{ls, read, refs, RefDir};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_fixture(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();

    let ls_err = ls(&project, Some("src/lib.rs")).unwrap_err().to_string();
    assert!(ls_err.contains("not indexed"), "{ls_err}");

    let read_err = read(&project, "src/lib.rs/add", false)
        .unwrap_err()
        .to_string();
    assert!(read_err.contains("not indexed"), "{read_err}");

    let ref_err = refs(&project, "add", RefDir::In).unwrap_err().to_string();
    assert!(ref_err.contains("not indexed"), "{ref_err}");

    let full = read(&project, "src/lib.rs", true).unwrap();
    assert!(full.source.contains("fn add"));
}

#[test]
fn empty_repo_without_sources_does_not_ask_to_index() {
    use cam::code::ls;
    use cam::project::Project;

    let dir = tempdir().unwrap();
    let project = Project::init(Some(dir.path())).unwrap();
    assert!(ls(&project, None).unwrap().is_empty());
}
