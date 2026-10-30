use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::tempdir;

fn cam_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_cam").into()
}

fn cam_json(project: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(project)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn entry_names(value: &serde_json::Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect()
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

fn write_polyglot_fixture(root: &std::path::Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    let files = [
        (
            "src/Foo.java",
            r#"
class Bar {}
class Foo extends Bar {
  void helper() {}
  void run() { helper(); }
}
"#,
        ),
        (
            "src/util.c",
            r#"
int add(int a, int b) { return a + b; }
int run(void) { return add(1, 2); }
"#,
        ),
        (
            "src/util.h",
            r#"
class Bar {};
class Foo : public Bar {
public:
  void helper();
};
"#,
        ),
        (
            "src/foo.cpp",
            r#"
#include "util.h"
void Foo::helper() {}
void run(Foo* f) { f->helper(); }
"#,
        ),
        (
            "src/Foo.cs",
            r#"
class Bar {}
class Foo : Bar {
  void Helper() {}
  void Run() { Helper(); }
}
"#,
        ),
    ];
    for (name, body) in files {
        fs::write(root.join(name), body).unwrap();
    }
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
fn index_java_c_cpp_csharp() {
    let dir = tempdir().unwrap();
    write_polyglot_fixture(dir.path());

    let report = cam_json(dir.path(), &["index"]);
    assert!(report["files"].as_u64().unwrap() >= 5, "{report}");

    let java = cam_json(dir.path(), &["ls", "src/Foo.java"]);
    let names = entry_names(&java);
    assert!(names.contains(&"Foo"), "{java}");
    assert!(names.contains(&"run"), "{java}");

    let callers = cam_json(dir.path(), &["ref", "src/Foo.java/helper", "--dir", "in"]);
    let names = entry_names(&callers["refs"]);
    assert!(names.contains(&"run"), "{callers}");

    let header = cam_json(dir.path(), &["ls", "src/util.h"]);
    let names = entry_names(&header);
    assert!(names.contains(&"Foo"), "{header}");

    let csharp = cam_json(dir.path(), &["ref", "src/Foo.cs/Helper", "--dir", "in"]);
    let names = entry_names(&csharp["refs"]);
    assert!(names.contains(&"Run"), "{csharp}");
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

fn write_find_fixture(root: &std::path::Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub struct Parser;\npub fn parse() {}\npub fn parse_ref() {}\npub fn parsexref() {}\npub fn reparse() {}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("lib/parser.rs"), "pub fn parser() {}\n").unwrap();
    fs::create_dir_all(root.join("py")).unwrap();
    fs::write(root.join("py/tool.py"), "def parse_ref():\n    pass\n").unwrap();
}

#[test]
fn find_ranks_name_substrings_and_applies_filters() {
    use cam::code::{find, index_project, read, ReadOutcome, SymbolHints};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_find_fixture(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let all = find(&project, "PARSE", SymbolHints::default(), 20).unwrap();
    let names: Vec<&str> = all.hits.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(all.total_matches, 7, "{names:?}");
    // Exact name, then prefixes (type-like kinds first), then inner substrings.
    assert_eq!(names[0], "parse", "{names:?}");
    assert_eq!(names[1], "Parser", "{names:?}");
    assert_eq!(names[6], "reparse", "{names:?}");

    // Same-case exact name beats a differently cased type, despite kind priority.
    let lower = find(&project, "parser", SymbolHints::default(), 20).unwrap();
    assert_eq!(lower.hits[0].name, "parser", "{lower:?}");
    assert_eq!(lower.hits[1].name, "Parser", "{lower:?}");

    // `_` is literal, not a LIKE wildcard.
    let underscored = find(&project, "parse_ref", SymbolHints::default(), 20).unwrap();
    assert_eq!(underscored.total_matches, 2, "{underscored:?}");
    assert!(underscored.hits.iter().all(|h| h.name == "parse_ref"));

    let structs = find(
        &project,
        "parse",
        SymbolHints {
            kind: Some("struct"),
            ..SymbolHints::default()
        },
        20,
    )
    .unwrap();
    assert_eq!(structs.total_matches, 1);
    assert_eq!(structs.hits[0].path, "src/lib.rs/Parser");

    let in_py = find(
        &project,
        "parse",
        SymbolHints {
            file: Some("PY/"),
            ..SymbolHints::default()
        },
        20,
    )
    .unwrap();
    assert_eq!(in_py.total_matches, 1);
    assert_eq!(in_py.hits[0].file_path, "py/tool.py");

    let limited = find(&project, "parse", SymbolHints::default(), 2).unwrap();
    assert_eq!(limited.total_matches, 7);
    assert_eq!(limited.hits.len(), 2);

    assert!(find(&project, "nothing_here", SymbolHints::default(), 20)
        .unwrap()
        .hits
        .is_empty());
    assert!(find(&project, "  ", SymbolHints::default(), 20).is_err());

    // A hit's virtual path reads straight back as the symbol body.
    match read(&project, &structs.hits[0].path, false).unwrap() {
        ReadOutcome::Ok(r) => assert!(r.source.contains("struct Parser"), "{r:?}"),
        other => panic!("expected ok, got {other:?}"),
    }

    let cli = cam_json(dir.path(), &["find", "parse_ref", "--scope", "src"]);
    assert_eq!(cli["total_matches"], 1, "{cli}");
    assert_eq!(cli["hits"][0]["path"], "src/lib.rs/parse_ref");
}

#[test]
fn find_without_index_asks_to_index() {
    use cam::code::{find, SymbolHints};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_find_fixture(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();
    let err = find(&project, "parse", SymbolHints::default(), 20).unwrap_err();
    assert!(err.to_string().contains("cam index"), "{err}");
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

fn sqlite_strings(db: &std::path::Path, sql: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

#[test]
fn nested_git_projects_keep_their_own_graph() {
    use cam::code::{index_project, ls, read, ReadOutcome};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_fixture(dir.path());
    // Standalone clone: .git is a directory.
    fs::create_dir_all(dir.path().join("clone/.git")).unwrap();
    fs::write(dir.path().join("clone/lib.rs"), "pub fn from_clone() {}\n").unwrap();
    // Checkout nested inside that clone.
    fs::create_dir_all(dir.path().join("clone/inner/.git")).unwrap();
    fs::write(
        dir.path().join("clone/inner/lib.rs"),
        "pub fn from_inner() {}\n",
    )
    .unwrap();
    // Submodule / linked worktree: .git is a file.
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();
    fs::write(dir.path().join("sub/lib.rs"), "pub fn from_sub() {}\n").unwrap();

    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let names = ls_root_names(&project);
    assert!(names.contains(&"clone".to_string()), "{names:?}");
    assert!(names.contains(&"sub".to_string()), "{names:?}");
    assert!(names.contains(&"src".to_string()), "{names:?}");

    let root_files = sqlite_strings(&dir.path().join(".cam/cam.db"), "SELECT path FROM files");
    assert!(
        root_files
            .iter()
            .all(|path| !path.starts_with("clone/") && !path.starts_with("sub/")),
        "{root_files:?}"
    );
    let catalog = sqlite_strings(
        &dir.path().join(".cam/cam.db"),
        "SELECT rel_path FROM code_projects ORDER BY rel_path",
    );
    assert_eq!(catalog, vec!["clone".to_string(), "sub".to_string()]);

    let clone_files = sqlite_strings(
        &dir.path().join("clone/.cam/cam.db"),
        "SELECT path FROM files ORDER BY path",
    );
    assert_eq!(clone_files, vec!["lib.rs".to_string()]);
    let clone_catalog = sqlite_strings(
        &dir.path().join("clone/.cam/cam.db"),
        "SELECT rel_path FROM code_projects",
    );
    assert_eq!(clone_catalog, vec!["inner".to_string()]);
    assert!(dir.path().join("sub/.cam/cam.db").is_file());
    assert!(dir.path().join("clone/inner/.cam/cam.db").is_file());

    let inside = ls(&project, Some("clone")).unwrap();
    assert!(
        inside.iter().any(|entry| entry.name == "lib.rs"),
        "{inside:?}"
    );
    let inner = inside.iter().find(|entry| entry.name == "inner").unwrap();
    assert_eq!(inner.kind, "project");

    match read(&project, "clone/lib.rs/from_clone", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("from_clone"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }
    match read(&project, "clone/inner/lib.rs/from_inner", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("from_inner"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }

    // Opening the nested checkout reuses the graph written by the parent index.
    let child = Project::init(Some(&dir.path().join("clone"))).unwrap();
    match read(&child, "lib.rs/from_clone", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("from_clone"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }
    match read(&child, "inner/lib.rs/from_inner", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("from_inner"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }
    let grandchild = Project::init(Some(&dir.path().join("clone/inner"))).unwrap();
    match read(&grandchild, "lib.rs/from_inner", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("from_inner"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// `tests/fixtures/umbrella` has no `.git` of its own. The test stamps the
/// checkout boundaries so the fixture can live inside this repository.
fn stage_umbrella(dst: &std::path::Path) {
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/umbrella");
    copy_dir(&src, dst);
    fs::create_dir_all(dst.join("app/.git")).unwrap();
    fs::create_dir_all(dst.join("app/inner/.git")).unwrap();
    fs::create_dir_all(dst.join("mirror/.git")).unwrap();
    fs::write(dst.join("sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();
}

#[test]
fn umbrella_fixture_matches_nested_graph_expectations() {
    use cam::code::{index_project, ls, read, refs, ReadOutcome, RefDir, RefOutcome, SymbolHints};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    stage_umbrella(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();
    index_project(&project).unwrap();

    let root_files = sqlite_strings(
        &dir.path().join(".cam/cam.db"),
        "SELECT path FROM files ORDER BY path",
    );
    assert_eq!(
        root_files,
        vec!["src/main.rs".to_string(), "tools/gen.py".to_string()],
        "parent graph keeps only files whose nearest .git is the umbrella"
    );
    let catalog = sqlite_strings(
        &dir.path().join(".cam/cam.db"),
        "SELECT rel_path FROM code_projects ORDER BY rel_path",
    );
    assert_eq!(
        catalog,
        vec!["app".to_string(), "mirror".to_string(), "sub".to_string()]
    );
    assert_eq!(
        sqlite_strings(
            &dir.path().join("app/.cam/cam.db"),
            "SELECT path FROM files ORDER BY path",
        ),
        vec!["src/config.rs".to_string()]
    );
    assert_eq!(
        sqlite_strings(
            &dir.path().join("app/.cam/cam.db"),
            "SELECT rel_path FROM code_projects ORDER BY rel_path",
        ),
        vec!["inner".to_string()]
    );
    assert_eq!(
        sqlite_strings(
            &dir.path().join("app/inner/.cam/cam.db"),
            "SELECT path FROM files",
        ),
        vec!["src/lib.rs".to_string()]
    );
    assert_eq!(
        sqlite_strings(
            &dir.path().join("mirror/.cam/cam.db"),
            "SELECT path FROM files",
        ),
        vec!["src/config.rs".to_string()]
    );
    assert_eq!(
        sqlite_strings(
            &dir.path().join("sub/.cam/cam.db"),
            "SELECT path FROM files"
        ),
        vec!["lib.rs".to_string()]
    );

    let listed = ls(&project, None).unwrap();
    let kind_of = |name: &str| {
        listed
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.kind.as_str())
            .unwrap_or("missing")
    };
    assert_eq!(kind_of("app"), "project", "{listed:?}");
    assert_eq!(kind_of("mirror"), "project", "{listed:?}");
    assert_eq!(kind_of("sub"), "project", "{listed:?}");
    assert_eq!(kind_of("tools"), "dir", "{listed:?}");
    assert_eq!(kind_of("src"), "dir", "{listed:?}");

    let bare = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints::default(),
    )
    .unwrap();
    match bare {
        RefOutcome::Ambiguous(ambiguous) => {
            assert_eq!(ambiguous.total_candidates, 3, "{ambiguous:?}");
        }
        other => panic!("expected ambiguous, got {other:?}"),
    }
    let scoped = refs(
        &project,
        "refresh_tools",
        RefDir::In,
        SymbolHints {
            scope: Some("app"),
            ..SymbolHints::default()
        },
    )
    .unwrap();
    match scoped {
        RefOutcome::Ok(result) => {
            assert_eq!(result.resolved.file_path, "app/src/config.rs");
            assert_eq!(result.refs.len(), 1, "{result:?}");
            assert_eq!(result.refs[0].name, "boot");
            assert_eq!(result.refs[0].file_path, "app/src/config.rs");
        }
        other => panic!("expected ok, got {other:?}"),
    }
    match read(&project, "app/inner/src/lib.rs/from_inner", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("from_inner"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }
    match read(&project, "src/main.rs/root_only", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("root_only"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }

    let app = Project::init(Some(&dir.path().join("app"))).unwrap();
    let in_app = refs(&app, "refresh_tools", RefDir::In, SymbolHints::default()).unwrap();
    match in_app {
        RefOutcome::Ok(result) => {
            assert_eq!(result.resolved.file_path, "src/config.rs");
            assert_eq!(result.refs.len(), 1, "{result:?}");
            assert_eq!(result.refs[0].file_path, "src/config.rs");
        }
        other => panic!("expected the child graph to resolve one definition, got {other:?}"),
    }
    assert!(
        refs(&app, "root_only", RefDir::In, SymbolHints::default()).is_err(),
        "a child checkout must not see symbols that belong to the parent"
    );
    match read(&app, "inner/src/lib.rs/from_inner", false).unwrap() {
        ReadOutcome::Ok(got) => assert!(got.source.contains("from_inner"), "{got:?}"),
        other => panic!("expected ok, got {other:?}"),
    }
}

#[test]
fn index_progress_visits_each_phase_in_order() {
    use cam::code::{index_project_with_progress, IndexPhase};
    use cam::project::Project;

    let dir = tempdir().unwrap();
    write_fixture(dir.path());
    let project = Project::init(Some(dir.path())).unwrap();
    let mut events = Vec::new();
    index_project_with_progress(&project, &mut |event| events.push(event)).unwrap();

    let rank = |phase: IndexPhase| match phase {
        IndexPhase::Scanning => 0,
        IndexPhase::Parsing => 1,
        IndexPhase::Storing => 2,
        IndexPhase::Resolving => 3,
    };
    assert!(events.len() > 4, "{events:?}");
    for pair in events.windows(2) {
        assert!(rank(pair[1].phase) >= rank(pair[0].phase));
    }
    for phase in [
        IndexPhase::Scanning,
        IndexPhase::Parsing,
        IndexPhase::Storing,
        IndexPhase::Resolving,
    ] {
        assert!(
            events.iter().any(|event| event.phase == phase),
            "{events:?}"
        );
    }
    let last_scan = events
        .iter()
        .rfind(|event| event.phase == IndexPhase::Scanning)
        .unwrap();
    assert_eq!(last_scan.total, 0);
    assert!(last_scan.current >= 2, "{last_scan:?}");
    for phase in [
        IndexPhase::Parsing,
        IndexPhase::Storing,
        IndexPhase::Resolving,
    ] {
        let last = events.iter().rfind(|event| event.phase == phase).unwrap();
        assert_eq!(last.current, last.total, "{last:?}");
        assert!(last.total >= 2, "{last:?}");
    }
}

#[test]
fn index_json_and_piped_text_hide_the_progress_bar() {
    let dir = tempdir().unwrap();
    write_fixture(dir.path());

    let json = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .arg("index")
        .output()
        .unwrap();
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let json_err = String::from_utf8_lossy(&json.stderr);
    assert!(!json_err.contains("Scanning"), "{json_err}");
    assert!(!json_err.contains("Indexing project"), "{json_err}");

    let fresh = tempdir().unwrap();
    write_fixture(fresh.path());
    let text = Command::new(cam_bin())
        .args(["--project"])
        .arg(fresh.path())
        .arg("index")
        .output()
        .unwrap();
    assert!(
        text.status.success(),
        "{}",
        String::from_utf8_lossy(&text.stderr)
    );
    assert!(
        text.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&text.stderr)
    );
    let stdout = String::from_utf8(text.stdout).unwrap();
    assert!(stdout.contains("indexed "), "{stdout}");
    assert!(!stdout.contains("Scanning"), "{stdout}");
}

#[test]
fn empty_repo_without_sources_does_not_ask_to_index() {
    use cam::code::ls;
    use cam::project::Project;

    let dir = tempdir().unwrap();
    let project = Project::init(Some(dir.path())).unwrap();
    assert!(ls(&project, None).unwrap().is_empty());
}
