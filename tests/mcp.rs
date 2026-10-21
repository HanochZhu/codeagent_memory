use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
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
        self.stdout.read_line(&mut line).expect("read mcp response");
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
    let version = resp["result"]["serverInfo"]["version"].as_str().unwrap();
    let (crate_version, hash) = version.split_once('+').expect(version);
    assert!(crate_version.split('.').count() >= 3, "{version}");
    assert!(
        hash == "unknown" || (hash.len() >= 12 && hash.chars().all(|c| c.is_ascii_hexdigit())),
        "{version}"
    );
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
    let recall_tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "cam_recall")
        .unwrap();
    assert!(recall_tool["inputSchema"]["properties"]
        .get("fusion")
        .is_none());

    let prompts = mcp.send(&json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "prompts/list"
    }));
    assert_eq!(prompts["result"]["prompts"].as_array().unwrap().len(), 2);
    let prompt = mcp.send(&rpc(
        21,
        "prompts/get",
        json!({
            "name": "remember_solution",
            "arguments": { "summary": "recall order", "body": "Call cam_recall before reading code." }
        }),
    ));
    assert!(prompt["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("cam_add"));

    let resources = mcp.send(&json!({
        "jsonrpc": "2.0",
        "id": 22,
        "method": "resources/list"
    }));
    assert_eq!(
        resources["result"]["resources"].as_array().unwrap().len(),
        2
    );
    let status = mcp.send(&rpc(23, "resources/read", json!({ "uri": "cam://status" })));
    let status_text = status["result"]["contents"][0]["text"].as_str().unwrap();
    let status_json: Value = serde_json::from_str(status_text).unwrap();
    assert_eq!(status_json["indexed"], false);
    assert_eq!(status["result"]["contents"][0]["uri"], "cam://status");

    let ping = mcp.send(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "ping"
    }));
    assert!(ping.get("result").is_some());
    assert!(!names.contains(&"cam_init"));

    let added_payload = call_ok(
        &mut mcp,
        5,
        "cam_add",
        json!({
            "path": root,
            "summary": "BM25 and vector hybrid recall",
            "body": "Use FTS5 BM25 plus cosine vectors, then apply reciprocal rank fusion.",
            "hash_embed": true
        }),
    );
    let id = added_payload["id"].as_str().unwrap().to_string();

    let revision = call_ok(
        &mut mcp,
        6,
        "cam_add",
        json!({
            "path": root,
            "summary": "English normalized hybrid recall",
            "body": "Use normalized English terms before fusing lexical and vector recall.",
            "supersedes": id,
            "hash_embed": true
        }),
    );
    let revision_id = revision["id"].as_str().unwrap().to_string();

    let hits = call_ok(
        &mut mcp,
        7,
        "cam_recall",
        json!({
            "path": root,
            "query": "How does BM25 and vector hybrid recall work?",
            "hash_embed": true
        }),
    );
    assert!(hits
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| hit["id"] == revision_id));
    assert!(!hits.as_array().unwrap().iter().any(|hit| hit["id"] == id));

    let history = call_ok(
        &mut mcp,
        8,
        "cam_recall",
        json!({
            "path": root,
            "query": "How does BM25 and vector hybrid recall work?",
            "include_superseded": true,
            "limit": 5,
            "hash_embed": true
        }),
    );
    assert!(history
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| hit["id"] == id && hit["latest"] == false));

    let nodes = call_ok(&mut mcp, 9, "cam_mem_tree", json!({ "path": root }));
    assert_eq!(nodes.as_array().unwrap().len(), 2);

    let non_latin_add = call_ok(
        &mut mcp,
        10,
        "cam_add",
        json!({
            "path": root,
            "summary": "Non-Latin body storage",
            "body": "支持在正文中使用非拉丁字符，例如中文。",
            "links": [revision_id],
            "hash_embed": true
        }),
    );
    let linked_id = non_latin_add["id"].as_str().unwrap().to_string();
    assert_eq!(non_latin_add["links"], json!([revision_id]));

    let shown = call_ok(
        &mut mcp,
        12,
        "cam_mem_show",
        json!({
            "path": root,
            "id": revision_id
        }),
    );
    assert_eq!(shown["links"], json!([linked_id]));

    let non_latin_summary = call_tool(
        &mut mcp,
        11,
        "cam_add",
        json!({
            "path": root,
            "summary": "非英文摘要",
            "body": "English body",
            "hash_embed": true
        }),
    );
    assert_eq!(non_latin_summary["result"]["isError"], true);
    assert!(non_latin_summary["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("summary must be written in English"));

    let batch = call_ok(
        &mut mcp,
        13,
        "cam_add",
        json!({
            "path": root,
            "hash_embed": true,
            "memories": [
                {
                    "summary": "The shell tool truncates output at 1000 lines",
                    "body": "formatLargeTextForModel keeps the last 1000 lines."
                },
                {
                    "summary": "Images use a separate size cap",
                    "body": "Images over 5 MB base64 are replaced with a not-sent note.",
                    "parent": "#0",
                    "links": ["#0"]
                }
            ]
        }),
    );
    let stored = batch["memories"].as_array().unwrap();
    assert_eq!(stored.len(), 2);
    let first = stored[0]["id"].as_str().unwrap();
    let second = stored[1]["id"].as_str().unwrap();
    assert_eq!(stored[1]["parent_id"].as_str(), Some(first));
    assert_eq!(stored[1]["links"], json!([first]));
    let shown = call_ok(
        &mut mcp,
        14,
        "cam_mem_show",
        json!({ "path": root, "id": first }),
    );
    assert_eq!(shown["links"], json!([second]));

    let rejected = call_tool(
        &mut mcp,
        15,
        "cam_add",
        json!({
            "path": root,
            "hash_embed": true,
            "summary": "Should not mix with memories",
            "body": "Detail.",
            "memories": [{
                "summary": "Another fact",
                "body": "Detail of that fact."
            }]
        }),
    );
    assert_eq!(rejected["result"]["isError"], true);
}

#[test]
fn mcp_cam_project_beats_server_path() {
    let server_dir = tempdir().unwrap();
    let env_dir = tempdir().unwrap();
    let mut mcp = McpChild::spawn_with(Some(server_dir.path()), Some(env_dir.path()));
    initialize(&mut mcp);

    call_ok(&mut mcp, 4, "cam_mem_tree", json!({}));
    assert!(env_dir.path().join(".cam/cam.db").is_file());
    assert!(!server_dir.path().join(".cam").exists());
}

#[test]
fn mcp_tool_path_beats_cam_project() {
    let env_dir = tempdir().unwrap();
    let tool_dir = tempdir().unwrap();
    let mut mcp = McpChild::spawn_with(None, Some(env_dir.path()));
    initialize(&mut mcp);

    call_ok(
        &mut mcp,
        4,
        "cam_mem_tree",
        json!({ "path": tool_dir.path().display().to_string() }),
    );
    assert!(tool_dir.path().join(".cam/cam.db").is_file());
    assert!(!env_dir.path().join(".cam").exists());
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
        None,
    )
    .unwrap();
    let config = format!("current_project = '{}'\n", old_project.path().display());
    std::fs::create_dir(home.path().join(".cam")).unwrap();
    std::fs::write(home.path().join(".cam/config.toml"), &config).unwrap();
    let mut mcp = McpChild::spawn_in(None, None, Some(cwd.path()), Some(home.path()));
    initialize(&mut mcp);

    let resp = call_tool(&mut mcp, 2, "cam_mem_tree", json!({}));
    if resp["result"]["isError"] == false {
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.contains(&added.id),
            "legacy current_project was used: {text}"
        );
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
fn mcp_first_tool_auto_inits_without_touching_global_config() {
    for config in [
        None,
        Some("stale_days = 7\ncurrent_project = '/old/project'\n"),
        Some("invalid = ["),
    ] {
        let home = tempdir().unwrap();
        let cwd = tempdir().unwrap();
        let config_path = home.path().join(".cam/config.toml");
        if let Some(text) = config {
            std::fs::create_dir(home.path().join(".cam")).unwrap();
            std::fs::write(&config_path, text).unwrap();
        }
        let mut mcp = McpChild::spawn_in(None, None, Some(cwd.path()), Some(home.path()));
        initialize(&mut mcp);

        let path = cwd.path().display().to_string();
        let first = call_ok(&mut mcp, 2, "cam_mem_tree", json!({ "path": path }));
        let second = call_ok(&mut mcp, 3, "cam_mem_tree", json!({ "path": path }));
        assert_eq!(first, second);
        assert!(cwd.path().join(".cam/cam.db").is_file());
        if let Some(text) = config {
            assert_eq!(std::fs::read_to_string(&config_path).unwrap(), text);
        } else {
            assert!(!home.path().join(".cam").exists());
        }
    }
}

#[test]
fn mcp_ls_without_index_asks_to_index() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn add() {}\n").unwrap();
    let root = dir.path().display().to_string();
    let mut mcp = McpChild::spawn(None);
    initialize(&mut mcp);

    let resp = call_tool(&mut mcp, 5, "cam_ls", json!({ "path": root }));
    assert_eq!(resp["result"]["isError"], true, "{resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("cam_index"), "{text}");
}

#[test]
fn mcp_ref_ambiguity_is_a_result_not_an_error() {
    let dir = tempdir().unwrap();
    for repo in ["app", "mirror"] {
        std::fs::create_dir_all(dir.path().join(repo).join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join(repo).join("src")).unwrap();
        std::fs::write(
            dir.path().join(repo).join("src/lib.rs"),
            "pub fn add() {}\npub fn run() { add(); }\n",
        )
        .unwrap();
    }
    let root = dir.path().display().to_string();
    let mut mcp = McpChild::spawn(None);
    initialize(&mut mcp);
    call_ok(&mut mcp, 2, "cam_index", json!({ "path": root }));

    let listed = call_ok(&mut mcp, 3, "cam_ls", json!({ "path": root }));
    let kinds: Vec<(&str, &str)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["name"].as_str().unwrap(), e["kind"].as_str().unwrap()))
        .collect();
    assert!(kinds.contains(&("app", "project")), "{listed}");
    assert!(kinds.contains(&("mirror", "project")), "{listed}");

    let ambiguous = call_ok(
        &mut mcp,
        4,
        "cam_ref",
        json!({ "path": root, "symbol": "add", "dir": "in" }),
    );
    assert_eq!(ambiguous["status"], "ambiguous", "{ambiguous}");
    assert_eq!(ambiguous["total_candidates"], 2);
    let first_id = ambiguous["candidates"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let by_id = call_ok(
        &mut mcp,
        5,
        "cam_ref",
        json!({ "path": root, "symbol": first_id, "dir": "in" }),
    );
    assert_eq!(by_id["status"], "ok", "{by_id}");
    assert_eq!(by_id["resolved"]["id"], first_id);
    assert_eq!(by_id["refs"][0]["name"], "run");

    let scoped = call_ok(
        &mut mcp,
        6,
        "cam_ref",
        json!({ "path": root, "symbol": "add", "dir": "in", "scope": "mirror" }),
    );
    assert_eq!(scoped["status"], "ok", "{scoped}");
    assert_eq!(scoped["resolved"]["file_path"], "mirror/src/lib.rs");

    let hinted = call_ok(
        &mut mcp,
        7,
        "cam_ref",
        json!({ "path": root, "symbol": "add", "dir": "in", "file": "app/", "kind": "function" }),
    );
    assert_eq!(
        hinted["resolved"]["file_path"], "app/src/lib.rs",
        "{hinted}"
    );
}

#[test]
fn mcp_find_returns_hits_that_read_back() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn load_config() {}\npub fn reload_config() { load_config(); }\npub struct Config;\n",
    )
    .unwrap();
    let root = dir.path().display().to_string();
    let mut mcp = McpChild::spawn(None);
    initialize(&mut mcp);

    let listed = mcp.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    assert!(
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "cam_find"),
        "{listed}"
    );

    call_ok(&mut mcp, 2, "cam_index", json!({ "path": root }));
    let found = call_ok(
        &mut mcp,
        3,
        "cam_find",
        json!({ "path": root, "query": "config", "kind": "function", "limit": 1 }),
    );
    assert_eq!(found["total_matches"], 2, "{found}");
    assert_eq!(found["hits"].as_array().unwrap().len(), 1);
    assert_eq!(found["hits"][0]["path"], "src/lib.rs/load_config");

    let body = call_ok(
        &mut mcp,
        4,
        "cam_read",
        json!({ "path": root, "virt_path": found["hits"][0]["id"] }),
    );
    assert_eq!(body["status"], "ok", "{body}");
    assert!(body["source"].as_str().unwrap().contains("fn load_config"));

    let missing = call_tool(&mut mcp, 5, "cam_find", json!({ "path": root }));
    assert_eq!(missing["result"]["isError"], true, "{missing}");
}

#[test]
fn mcp_unknown_tool_is_error() {
    let mut mcp = McpChild::spawn(None);
    initialize(&mut mcp);
    let resp = call_tool(&mut mcp, 9, "not_a_tool", json!({}));
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn mcp_help_lists_command() {
    let out = Command::new(cam_bin()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mcp"), "{text}");
    assert!(
        !text.lines().any(|line| line.trim().starts_with("init ")),
        "{text}"
    );
}
