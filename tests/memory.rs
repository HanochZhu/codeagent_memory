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

    let mut add = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args([
            "add",
            "--summary",
            "BM25 and vector hybrid recall",
            "--hash-embed",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    add.stdin
        .as_mut()
        .unwrap()
        .write_all(b"Use FTS5 BM25 plus cosine vectors, then apply reciprocal rank fusion.")
        .unwrap();
    let out = add.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let added: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let parent = added["id"].as_str().unwrap().to_string();

    let mut child = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args([
            "add",
            "--summary",
            "English terms are normalized before FTS",
            "--supersedes",
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
        .write_all(b"Stem English terms so BM25 can match inflected query words.")
        .unwrap();
    let revision = child.wait_with_output().unwrap();
    assert!(revision.status.success());
    let revision: serde_json::Value = serde_json::from_slice(&revision.stdout).unwrap();
    let revision_id = revision["id"].as_str().unwrap().to_string();

    let recall = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args([
            "recall",
            "How does BM25 and vector hybrid recall work?",
            "--hash-embed",
        ])
        .output()
        .unwrap();
    assert!(
        recall.status.success(),
        "{}",
        String::from_utf8_lossy(&recall.stderr)
    );
    let hits: Vec<serde_json::Value> = serde_json::from_slice(&recall.stdout).unwrap();
    assert!(!hits.is_empty());
    assert!(!hits.iter().any(|hit| hit["id"] == parent), "{hits:?}");
    assert!(!hits[0]["path"].as_array().unwrap().is_empty());
    assert!(hits[0].get("stale").is_some());
    assert!(hits[0].get("needs_update").is_some());
    assert!(hits[0]["retention"].as_f64().unwrap() > 0.9);

    // Default recall folds the superseded revision before top-k.
    let latest: Vec<&str> = hits
        .iter()
        .filter(|h| h["latest"] == true)
        .filter_map(|h| h["id"].as_str())
        .collect();
    assert_eq!(latest, vec![revision_id.as_str()], "{hits:?}");

    let tree = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["mem", "tree"])
        .output()
        .unwrap();
    assert!(tree.status.success());
    let nodes: Vec<serde_json::Value> = serde_json::from_slice(&tree.stdout).unwrap();
    assert_eq!(nodes.len(), 2, "revisions share a structural position");
    assert!(nodes
        .iter()
        .all(|node| node["children"].as_array().unwrap().is_empty()));

    let show = Command::new(cam_bin())
        .args(["--json", "--project"])
        .arg(dir.path())
        .args(["mem", "show", &revision_id])
        .output()
        .unwrap();
    assert!(show.status.success());
    let view: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(view["supersedes_id"], parent);
    assert_eq!(view["latest"], true);
}

#[test]
fn rejects_non_english_recall_and_memory_fields() {
    let dir = tempdir().unwrap();

    for args in [
        vec!["recall", "如何进行向量召回", "--hash-embed"],
        vec![
            "add",
            "--summary",
            "非英文摘要",
            "--body",
            "English body",
            "--hash-embed",
        ],
        vec![
            "add",
            "--summary",
            "English summary",
            "--body",
            "非英文正文",
            "--hash-embed",
        ],
    ] {
        let output = Command::new(cam_bin())
            .args(["--project"])
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("must be written in English"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
