use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn cam_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_cam").into()
}

fn output(home: &Path, args: &[&str]) -> Output {
    Command::new(cam_bin())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CAM_HASH_EMBED", "1")
        .env_remove("CAM_PROJECT")
        .args(args)
        .output()
        .unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout is not json ({err}): {} / stderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn write_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub fn run() { let _ = add(1, 2); }\n",
    )
    .unwrap();
}

#[test]
fn usage_error_is_json_on_stdout() {
    let home = tempdir().unwrap();
    let out = output(home.path(), &["--json", "index", "unexpected-positional"]);
    assert_eq!(out.status.code(), Some(2));
    let value = json(&out);
    assert_eq!(value["error"]["code"], "usage", "{value}");
}

#[test]
fn runtime_error_is_json_on_stdout() {
    let home = tempdir().unwrap();
    let root = tempdir().unwrap();

    let out = output(
        home.path(),
        &[
            "--json",
            "--project",
            root.path().to_str().unwrap(),
            "ref",
            "no_such_symbol",
            "--dir",
            "in",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let value = json(&out);
    assert_eq!(value["error"]["code"], "not_found", "{value}");
}

#[test]
fn json_is_compact_unless_pretty() {
    let home = tempdir().unwrap();
    let root = tempdir().unwrap();
    let project = root.path().to_str().unwrap();

    let compact = output(
        home.path(),
        &["--json", "--project", project, "status"],
    );
    assert!(compact.status.success());
    assert_eq!(
        String::from_utf8_lossy(&compact.stdout).trim().lines().count(),
        1
    );

    let pretty = output(
        home.path(),
        &["--json", "--pretty", "--project", project, "status"],
    );
    assert!(pretty.status.success());
    assert!(
        String::from_utf8_lossy(&pretty.stdout).trim().lines().count() > 1
    );
}

#[test]
fn add_body_status_and_ref_alias() {
    let home = tempdir().unwrap();
    let root = tempdir().unwrap();
    write_fixture(root.path());
    let project = root.path().to_str().unwrap();

    let indexed = output(
        home.path(),
        &["--json", "--project", project, "index"],
    );
    assert!(indexed.status.success(), "{}", String::from_utf8_lossy(&indexed.stderr));

    let added = output(
        home.path(),
        &[
            "--json",
            "--project",
            project,
            "add",
            "--summary",
            "inline body",
            "--body",
            "body text",
        ],
    );
    assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
    assert!(json(&added)["id"].as_str().is_some());

    let status = output(
        home.path(),
        &["--json", "--project", project, "status"],
    );
    let value = json(&status);
    assert_eq!(value["db_exists"], true, "{value}");
    assert_eq!(value["solutions"], 1, "{value}");
    assert!(value["nodes"].as_i64().unwrap() >= 2, "{value}");

    let callers = output(
        home.path(),
        &["--json", "--project", project, "ref", "add", "--callers"],
    );
    assert!(callers.status.success(), "{}", String::from_utf8_lossy(&callers.stderr));
    let callers_json = json(&callers);
    let names: Vec<_> = callers_json["refs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(names.contains(&"run"), "{names:?}");
}

#[test]
fn config_set_and_get_roundtrip() {
    let home = tempdir().unwrap();

    let set = output(home.path(), &["--json", "config", "set", "stale_days", "7"]);
    assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
    assert_eq!(json(&set)["stale_days"], 7);

    let get = output(home.path(), &["--json", "config", "get"]);
    assert_eq!(json(&get)["stale_days"], 7);

    let bad = output(home.path(), &["--json", "config", "set", "unknown", "1"]);
    assert_eq!(bad.status.code(), Some(1));
    assert_eq!(json(&bad)["error"]["code"], "usage", "{}", String::from_utf8_lossy(&bad.stdout));
}

#[test]
fn first_command_auto_inits() {
    let home = tempdir().unwrap();
    let root = tempdir().unwrap();
    write_fixture(root.path());
    let project = root.path().to_str().unwrap();
    assert!(!root.path().join(".cam").exists());

    let indexed = output(
        home.path(),
        &["--json", "--project", project, "index"],
    );
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    assert!(root.path().join(".cam/cam.db").is_file());
    assert!(json(&indexed)["files"].as_i64().unwrap() >= 1);
}
