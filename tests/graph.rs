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
        .args(["--json", "--path"])
        .arg(dir.path())
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());

    let out = Command::new(cam_bin())
        .args(["--json", "--path"])
        .arg(dir.path())
        .arg("index")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(report["files"].as_u64().unwrap() >= 2);
    assert!(report["nodes"].as_u64().unwrap() >= 6);

    let ls = Command::new(cam_bin())
        .args(["--json", "--path"])
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
        .args(["--json", "--path"])
        .arg(dir.path())
        .args(["read", "src/lib.rs/add"])
        .output()
        .unwrap();
    assert!(read.status.success());
    let body: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
    assert!(body["source"].as_str().unwrap().contains("fn add"));

    let callers = Command::new(cam_bin())
        .args(["--json", "--path"])
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
        .args(["--json", "--path"])
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
