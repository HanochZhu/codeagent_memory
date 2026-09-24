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

    let out = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("index")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
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
    let names: Vec<_> = entries.iter().filter_map(|e| e["name"].as_str()).collect();
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
    assert!(
        callers.status.success(),
        "{}",
        String::from_utf8_lossy(&callers.stderr)
    );
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

    let index = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("index")
        .output()
        .unwrap();
    assert!(
        index.status.success(),
        "{}",
        String::from_utf8_lossy(&index.stderr)
    );

    let noop = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("sync")
        .output()
        .unwrap();
    assert!(
        noop.status.success(),
        "{}",
        String::from_utf8_lossy(&noop.stderr)
    );
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
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );
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
    let names: Vec<_> = entries.iter().filter_map(|e| e["name"].as_str()).collect();
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
    use cam::code::{ls, read, refs, ReadOutcome, RefDir, SymbolHints};
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

    let ref_err = refs(&project, "add", RefDir::In, SymbolHints::default())
        .unwrap_err()
        .to_string();
    assert!(ref_err.contains("not indexed"), "{ref_err}");

    match read(&project, "src/lib.rs", true).unwrap() {
        ReadOutcome::Ok(full) => assert!(full.source.contains("fn add")),
        other => panic!("expected full read, got {other:?}"),
    }
}

/// Two nested repos with identical files, like a standalone clone next to a
/// checkout of the same repo, plus an unrelated helper of the same name.
fn write_multi_repo_fixture(root: &std::path::Path) {
    for repo in ["app", "mirror"] {
        fs::create_dir_all(root.join(repo).join(".git")).unwrap();
        fs::create_dir_all(root.join(repo).join("src")).unwrap();
        fs::write(
            root.join(repo).join("src/config.rs"),
            "pub fn refresh_tools() {}\npub fn boot() { refresh_tools(); }\n",
        )
        .unwrap();
    }
    fs::create_dir_all(root.join("tools")).unwrap();
    fs::write(
        root.join("tools/gen.py"),
        "class refresh_tools:\n    pass\n\ndef main():\n    refresh_tools()\n",
    )
    .unwrap();
}

#[test]
fn ambiguous_ref_returns_candidates_and_hints_resolve() {
    use cam::code::{index_project, refs, RefDir, RefOutcome, SymbolHints};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_multi_repo_fixture(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    // Bare name: three definitions -> ambiguous, not an error.
    let outcome = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints::default(),
    )
    .unwrap();
    let ambiguous = match outcome {
        RefOutcome::Ambiguous(a) => a,
        RefOutcome::Ok(r) => panic!("expected ambiguous, got {r:?}"),
    };
    assert_eq!(ambiguous.total_candidates, 3, "{ambiguous:?}");
    assert_eq!(ambiguous.candidates.len(), 3);
    // Without a kind hint, type-like kinds rank first.
    assert_eq!(ambiguous.candidates[0].kind, "class", "{ambiguous:?}");
    assert!(ambiguous.message.contains("re-call with `id`"));

    // A candidate id from the answer is the zero-ambiguity form.
    let by_id = ambiguous
        .candidates
        .iter()
        .find(|c| c.file_path == "app/src/config.rs")
        .unwrap();
    let resolved = match refs(&project, &by_id.id, RefDir::In, SymbolHints::default()).unwrap() {
        RefOutcome::Ok(r) => r,
        other => panic!("expected ok, got {other:?}"),
    };
    assert_eq!(resolved.resolved.id, by_id.id);
    assert_eq!(resolved.refs.len(), 1, "{resolved:?}");
    assert_eq!(resolved.refs[0].name, "boot");
    assert_eq!(resolved.refs[0].file_path, "app/src/config.rs");

    // file hint (substring, case-insensitive) picks the single match.
    let hinted = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints {
            file: Some("MIRROR/src"),
            ..SymbolHints::default()
        },
    )
    .unwrap();
    match hinted {
        RefOutcome::Ok(r) => assert_eq!(r.resolved.file_path, "mirror/src/config.rs"),
        other => panic!("expected ok, got {other:?}"),
    }

    // kind hint alone still leaves two functions -> ambiguous over the matches only.
    let by_kind = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints {
            kind: Some("function"),
            ..SymbolHints::default()
        },
    )
    .unwrap();
    match by_kind {
        RefOutcome::Ambiguous(a) => {
            assert_eq!(a.total_candidates, 2, "{a:?}");
            assert!(a.candidates.iter().all(|c| c.kind == "function"));
        }
        other => panic!("expected ambiguous, got {other:?}"),
    }

    // Hints that match nothing report every candidate so the caller can fix them.
    let wrong = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints {
            file: Some("does-not-exist"),
            ..SymbolHints::default()
        },
    )
    .unwrap();
    match wrong {
        RefOutcome::Ambiguous(a) => {
            assert_eq!(a.total_candidates, 3);
            assert!(a.message.contains("no candidate matched"), "{}", a.message);
        }
        other => panic!("expected ambiguous, got {other:?}"),
    }
}

#[test]
fn ls_marks_nested_repos_and_scope_filters_refs() {
    use cam::code::{index_project, ls, refs, RefDir, RefOutcome, SymbolHints};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_multi_repo_fixture(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let root = ls(&project, None).unwrap();
    let kind_of = |name: &str| {
        root.iter()
            .find(|e| e.name == name)
            .map(|e| e.kind.as_str())
    };
    assert_eq!(kind_of("app"), Some("project"), "{root:?}");
    assert_eq!(kind_of("mirror"), Some("project"), "{root:?}");
    assert_eq!(kind_of("tools"), Some("dir"), "{root:?}");

    let scoped = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints {
            scope: Some("app/"),
            ..SymbolHints::default()
        },
    )
    .unwrap();
    let result = match scoped {
        RefOutcome::Ok(r) => r,
        other => panic!("expected ok, got {other:?}"),
    };
    assert_eq!(result.resolved.file_path, "app/src/config.rs");
    assert!(
        result.refs.iter().all(|r| r.file_path.starts_with("app/")),
        "{result:?}"
    );

    // Scope that matches nothing falls back to the full candidate list.
    let none = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints {
            scope: Some("nope"),
            ..SymbolHints::default()
        },
    )
    .unwrap();
    assert!(
        matches!(none, RefOutcome::Ambiguous(ref a) if a.total_candidates == 3),
        "{none:?}"
    );
}

#[test]
fn read_reports_same_name_twice_in_file_and_accepts_node_id() {
    use cam::code::{index_project, read, ReadOutcome};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        r#"
pub struct A;
pub struct B;
impl A { pub fn new() -> Self { A } }
impl B { pub fn new() -> Self { B } }
"#,
    )
    .unwrap();
    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let ambiguous = match read(&project, "src/lib.rs/new", false).unwrap() {
        ReadOutcome::Ambiguous(a) => a,
        other => panic!("expected ambiguous, got {other:?}"),
    };
    assert_eq!(ambiguous.total_candidates, 2, "{ambiguous:?}");
    let second = ambiguous
        .candidates
        .iter()
        .max_by_key(|c| c.start_line)
        .unwrap();
    match read(&project, &second.id, false).unwrap() {
        ReadOutcome::Ok(r) => {
            assert_eq!(r.start_line, Some(second.start_line));
            assert!(r.source.contains("B }"), "{}", r.source);
        }
        other => panic!("expected ok, got {other:?}"),
    }

    // The JSON shape carries the status tag for both variants.
    let out = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["read", "src/lib.rs/new"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["status"], "ambiguous");
    assert_eq!(value["candidates"].as_array().unwrap().len(), 2);
}

fn ls_root_names(project: &cam::project::Project) -> Vec<String> {
    cam::code::ls(project, None)
        .unwrap()
        .iter()
        .map(|e| e.name.clone())
        .collect()
}

#[test]
fn camignore_and_root_gitignore_apply_without_git_repo() {
    use cam::code::index_project;
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_fixture(dir.path());
    fs::create_dir_all(dir.path().join("dup/src")).unwrap();
    fs::write(dir.path().join("dup/src/lib.rs"), "pub fn add() {}\n").unwrap();
    fs::create_dir_all(dir.path().join("gen")).unwrap();
    fs::write(dir.path().join("gen/out.py"), "def add():\n    pass\n").unwrap();
    fs::write(dir.path().join(".camignore"), "dup/\n").unwrap();
    // No .git at the root: the ignore file must still be honored.
    fs::write(dir.path().join(".gitignore"), "gen/\n").unwrap();

    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let names = ls_root_names(&project);
    assert!(names.contains(&"src".to_string()), "{names:?}");
    assert!(!names.contains(&"dup".to_string()), "{names:?}");
    assert!(!names.contains(&"gen".to_string()), "{names:?}");
}

#[test]
fn submodule_checkouts_are_skipped_but_nested_clones_stay() {
    use cam::code::index_project;
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_fixture(dir.path());
    // Standalone clone: .git is a directory -> indexed.
    fs::create_dir_all(dir.path().join("clone/.git")).unwrap();
    fs::write(dir.path().join("clone/lib.rs"), "pub fn from_clone() {}\n").unwrap();
    // Submodule / linked worktree: .git is a file -> skipped.
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();
    fs::write(dir.path().join("sub/lib.rs"), "pub fn from_sub() {}\n").unwrap();

    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let names = ls_root_names(&project);
    assert!(names.contains(&"clone".to_string()), "{names:?}");
    assert!(!names.contains(&"sub".to_string()), "{names:?}");
}

#[test]
fn empty_repo_without_sources_does_not_ask_to_index() {
    use cam::code::ls;
    use cam::project::Project;

    let dir = tempdir().unwrap();
    let project = Project::init(Some(dir.path())).unwrap();
    assert!(ls(&project, None).unwrap().is_empty());
}
