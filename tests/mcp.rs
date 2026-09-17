use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use serde_json::{json, Value};
use tempfile::tempdir;

fn cam_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_cam").into()
}

struct McpChild {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl McpChild {
    fn spawn(default_path: Option<&std::path::Path>) -> Self {
        Self::spawn_with(default_path, None)
    }

    fn spawn_with(
        default_path: Option<&std::path::Path>,
        cam_project: Option<&std::path::Path>,
    ) -> Self {
        Self::spawn_in(default_path, cam_project, None, None)
    }

    fn spawn_in(
        default_path: Option<&std::path::Path>,
        cam_project: Option<&std::path::Path>,
        cwd: Option<&std::path::Path>,
        home: Option<&std::path::Path>,
    ) -> Self {
        let mut cmd = Command::new(cam_bin());
        if let Some(path) = cwd {
            cmd.current_dir(path);
        }
        if let Some(path) = home {
            cmd.env("HOME", path).env("USERPROFILE", path);
        }
        cmd.arg("mcp")
            .env_remove("CAM_PROJECT")
            .env("CAM_HASH_EMBED", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = default_path {
            cmd.arg("--project").arg(path);
        }
        if let Some(path) = cam_project {
            cmd.env("CAM_PROJECT", path);
        }
        let mut child = cmd.spawn().expect("spawn cam mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, req: &Value) -> Value {
        writeln!(self.stdin, "{}", req).expect("write mcp request");
        self.stdin.flush().expect("flush mcp request");
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("read mcp response");
        assert!(
            !line.trim().is_empty(),
            "mcp server closed stdout without a response"
        );
        serde_json::from_str(line.trim()).unwrap_or_else(|err| {
            panic!("invalid mcp json {err}: {line}");
        })
    }

    fn notify(&mut self, req: &Value) {
        writeln!(self.stdin, "{}", req).expect("write mcp notification");
        self.stdin.flush().expect("flush mcp notification");
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn rpc(id: u64, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
}

fn initialize(mcp: &mut McpChild) {
    let resp = mcp.send(&rpc(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "cam-test", "version": "0" }
        }),
    ));
    assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(resp["result"]["serverInfo"]["name"], "cam");
    mcp.notify(&json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));
}

fn call_tool(mcp: &mut McpChild, id: u64, name: &str, arguments: Value) -> Value {
    mcp.send(&rpc(
        id,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    ))
}

fn tool_payload(resp: &Value) -> Value {
    assert_eq!(resp["result"]["isError"], false, "{resp}");
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text");
    serde_json::from_str(text).expect("tool payload json")
}

fn call_ok(mcp: &mut McpChild, id: u64, name: &str, arguments: Value) -> Value {
    tool_payload(&call_tool(mcp, id, name, arguments))
}

#[test]
fn mcp_stdio_init_add_recall() {
    let dir = tempdir().unwrap();
    let root = dir.path().display().to_string();
    let mut mcp = McpChild::spawn(Some(dir.path()));
    initialize(&mut mcp);

    let listed = mcp.send(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }));
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"cam_recall"));
    assert!(names.contains(&"cam_add"));

    let ping = mcp.send(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "ping"
    }));
    assert!(ping.get("result").is_some());

    let init_payload = call_ok(
        &mut mcp,
        4,
        "cam_init",
        json!({ "path": root }),
    );
    assert!(init_payload["root"].as_str().is_some());

    let added_payload = call_ok(
        &mut mcp,
        5,
        "cam_add",
        json!({
            "path": root,
            "summary": "BM25 与向量多路召回",
            "body": "Use FTS5 BM25 plus cosine vectors, min-max each path, then sum scores.",
            "hash_embed": true
        }),
    );
    let id = added_payload["id"].as_str().unwrap();

    let hits = call_ok(
        &mut mcp,
        6,
        "cam_recall",
        json!({
            "path": root,
            "query": "如何做 BM25 和向量的多路召回",
            "hash_embed": true
        }),
    );
    assert!(hits.as_array().unwrap().iter().any(|h| h["id"] == id));

    let nodes = call_ok(
        &mut mcp,
        7,
        "cam_mem_tree",
        json!({ "path": root }),
    );
    assert_eq!(nodes.as_array().unwrap().len(), 1);
}

#[test]
fn mcp_cam_project_beats_server_path() {
    let server_dir = tempdir().unwrap();
    let env_dir = tempdir().unwrap();
    let mut mcp = McpChild::spawn_with(Some(server_dir.path()), Some(env_dir.path()));
    initialize(&mut mcp);

    let payload = call_ok(&mut mcp, 4, "cam_init", json!({}));
    let root = payload["root"].as_str().unwrap();
    assert_eq!(
        std::fs::canonicalize(root).unwrap(),
        std::fs::canonicalize(env_dir.path()).unwrap()
    );
}

#[test]
fn mcp_tool_path_beats_cam_project() {
    let env_dir = tempdir().unwrap();
    let tool_dir = tempdir().unwrap();
    let mut mcp = McpChild::spawn_with(None, Some(env_dir.path()));
    initialize(&mut mcp);

    let payload = call_ok(
        &mut mcp,
        4,
        "cam_init",
        json!({ "path": tool_dir.path().display().to_string() }),
    );
    let root = payload["root"].as_str().unwrap();
    assert_eq!(
        std::fs::canonicalize(root).unwrap(),
        std::fs::canonicalize(tool_dir.path()).unwrap()
    );
}

#[test]
fn mcp_ignores_legacy_current_project() {
    let home = tempdir().unwrap();
    let cwd = tempdir().unwrap();
    let old_project = tempdir().unwrap();
    let legacy = cam::project::Project::init(Some(old_project.path())).unwrap();
    let embedder = cam::memory::HashEmbedder::default();
    let added = cam::memory::add_solution(
        &legacy,
        &embedder,
        "legacy memory that must not surface",
        "old body",
        None,
    )
    .unwrap();
    let config = format!(
        "current_project = '{}'\n",
        old_project.path().display()
    );
    std::fs::create_dir(home.path().join(".cam")).unwrap();
    std::fs::write(home.path().join(".cam/config.toml"), &config).unwrap();
    let mut mcp = McpChild::spawn_in(None, None, Some(cwd.path()), Some(home.path()));
    initialize(&mut mcp);

    let resp = call_tool(&mut mcp, 2, "cam_mem_tree", json!({}));
    if resp["result"]["isError"] == false {
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains(&added.id), "legacy current_project was used: {text}");
        assert!(
            !text.contains("legacy memory that must not surface"),
            "legacy current_project was used: {text}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(home.path().join(".cam/config.toml")).unwrap(),
        config
    );
}

#[test]
fn mcp_init_does_not_read_or_write_global_config() {
    for config in [None, Some("stale_days = 7\ncurrent_project = '/old/project'\n"), Some("invalid = [")] {
        let home = tempdir().unwrap();
        let cwd = tempdir().unwrap();
        let config_path = home.path().join(".cam/config.toml");
        if let Some(text) = config {
            std::fs::create_dir(home.path().join(".cam")).unwrap();
            std::fs::write(&config_path, text).unwrap();
        }
        let mut mcp = McpChild::spawn_in(None, None, Some(cwd.path()), Some(home.path()));
        initialize(&mut mcp);

        let first = call_ok(&mut mcp, 2, "cam_init", json!({}));
        let second = call_ok(&mut mcp, 3, "cam_init", json!({}));
        assert_eq!(first, second);
        assert_eq!(
            std::fs::canonicalize(first["root"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(cwd.path()).unwrap()
        );
        assert!(cwd.path().join(".cam/cam.db").is_file());
        if let Some(text) = config {
            assert_eq!(std::fs::read_to_string(&config_path).unwrap(), text);
        } else {
            assert!(!home.path().join(".cam").exists());
        }
    }
}

#[test]
fn mcp_unknown_tool_is_error() {
    let mut mcp = McpChild::spawn(None);
    initialize(&mut mcp);
    let resp = call_tool(
        &mut mcp,
        9,
        "not_a_tool",
        json!({}),
    );
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn mcp_help_lists_command() {
    let out = Command::new(cam_bin())
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mcp"), "{text}");
}
