use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use tempfile::tempdir;

fn cam_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_cam").into()
}

#[test]
fn add_and_recall_tree() {
    let dir = tempdir().unwrap();
    let init = Command::new(cam_bin())
        .args(["--path"])
        .arg(dir.path())
        .arg("init")
        .status()
        .unwrap();
    assert!(init.success());

    let mut add = Command::new(cam_bin())
        .args(["--json", "--path"])
        .arg(dir.path())
        .args(["add", "--summary", "BM25 与向量多路召回", "--hash-embed"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    add.stdin
        .as_mut()
        .unwrap()
        .write_all(b"Use FTS5 BM25 plus cosine vectors, min-max each path, then sum scores.")
        .unwrap();
    let out = add.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let added: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let parent = added["id"].as_str().unwrap().to_string();

    let mut child = Command::new(cam_bin())
        .args(["--json", "--path"])
        .arg(dir.path())
        .args([
            "add",
            "--summary",
            "jieba tokenizes Chinese before FTS",
            "--parent",
            &parent,
            "--hash-embed",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"Cut CJK with jieba so BM25 can match Chinese queries.")
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());

    let recall = Command::new(cam_bin())
        .args(["--json", "--path"])
        .arg(dir.path())
        .args([
            "recall",
            "如何做 BM25 和向量的多路召回",
            "--hash-embed",
        ])
        .output()
        .unwrap();
    assert!(recall.status.success(), "{}", String::from_utf8_lossy(&recall.stderr));
    let hits: Vec<serde_json::Value> = serde_json::from_slice(&recall.stdout).unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0]["path"].as_array().unwrap().len() >= 1);
    assert!(hits[0].get("stale").is_some());
    assert!(hits[0].get("needs_update").is_some());
    assert!(hits[0]["retention"].as_f64().unwrap() > 0.9);
    assert_eq!(hits[0]["latest"], true);

    let tree = Command::new(cam_bin())
        .args(["--json", "--path"])
        .arg(dir.path())
        .args(["mem", "tree"])
        .output()
        .unwrap();
    assert!(tree.status.success());
    let nodes: Vec<serde_json::Value> = serde_json::from_slice(&tree.stdout).unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["children"].as_array().unwrap().len(), 1);
}
