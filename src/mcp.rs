use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

use crate::code::{index_project, ls, read, refs, RefDir};
use crate::memory::{add_solution, show_solution, solution_tree, Embedder, Fusion, RecallOptions};
use crate::ops::{load_embedder, resolve_project_from_strings};
use crate::project::Project;

const SERVER_NAME: &str = "cam";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];
const FALLBACK_PROTOCOL_VERSION: &str = "2025-03-26";

const INSTRUCTIONS: &str = r#"cam is local code-graph + solution memory for this repo (SQLite under .cam/).

Main agent: call these MCP tools. Do not shell out to `cam` unless a tool is missing or fails.

Workflow:
1. cam_recall first. Reuse a hit when it is latest and not needs_update.
2. Before reading code, make sure the graph is indexed: if cam_ls / cam_read / cam_ref report an empty or not-indexed graph, call cam_index once for that project, then retry. cam_index rebuilds the whole graph, so use `cam sync` (CLI) for later edits instead of indexing again.
3. On miss / stale / needs_update: cam_ls → cam_read (prefer symbol paths) → cam_ref.
4. After you solve it, cam_add (summary + full body). Use parent to extend an older node.

Subagents usually have no MCP. Instruct them to run the equivalent CLI with --json in the project directory (see each tool description), including `cam index` once if the graph is not built.

Virtual paths: `src/main.rs` is a file; `src/main.rs/main` is a symbol in that file.
Project resolution: tool argument `path`, else CAM_PROJECT, else server `--project`, else .cam / .git walk-up from the server cwd, else the server cwd. `.cam/` and the database are created automatically on the first tool call."#;

#[derive(Debug, Clone, Default)]
pub struct McpContext {
    pub default_path: Option<PathBuf>,
}

pub fn serve_stdio(default_path: Option<&Path>) -> Result<()> {
    let ctx = McpContext {
        default_path: default_path.map(Path::to_path_buf),
    };
    eprintln!(
        "cam mcp {} ready (stdio)",
        env!("CARGO_PKG_VERSION")
    );

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if let Some(resp) = handle_message(&line, &ctx) {
            writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

fn handle_message(raw: &str, ctx: &McpContext) -> Option<JsonRpcResponse> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return Some(error_response(Value::Null, -32700, "Parse error")),
    };

    if parsed.is_array() {
        return Some(error_response(
            Value::Null,
            -32600,
            "JSON-RPC batch is not supported",
        ));
    }

    let Some(obj) = parsed.as_object() else {
        return Some(invalid_request(Value::Null));
    };

    let id = obj.get("id").cloned().unwrap_or(Value::Null);
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(invalid_request(id));
    }

    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Some(invalid_request(id));
    };

    if matches!(obj.get("id"), None | Some(Value::Null)) {
        let _ = dispatch(method, obj.get("params"), ctx);
        return None;
    }

    if !id.is_string() && !id.is_number() {
        return Some(error_response(
            Value::Null,
            -32600,
            "Invalid Request: id must be string or number",
        ));
    }

    match dispatch(method, obj.get("params"), ctx) {
        Ok(result) => Some(ok_response(id, result)),
        Err(DispatchError::MethodNotFound(msg)) => Some(error_response(id, -32601, msg)),
        Err(DispatchError::InvalidParams(msg)) => Some(error_response(id, -32602, msg)),
    }
}

enum DispatchError {
    MethodNotFound(String),
    InvalidParams(String),
}

fn dispatch(
    method: &str,
    params: Option<&Value>,
    ctx: &McpContext,
) -> Result<Value, DispatchError> {
    match method {
        "initialize" => Ok(initialize_result(params)),
        "ping" => Ok(json!({})),
        "notifications/initialized" | "notifications/cancelled" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_defs() })),
        "tools/call" => call_tool(params.unwrap_or(&Value::Null), ctx),
        other => Err(DispatchError::MethodNotFound(format!(
            "Method not found: {other}"
        ))),
    }
}

fn initialize_result(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str);
    let protocol_version = requested
        .filter(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(FALLBACK_PROTOCOL_VERSION);
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": INSTRUCTIONS
    })
}

fn call_tool(params: &Value, ctx: &McpContext) -> Result<Value, DispatchError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::InvalidParams("tools/call requires params.name".into()))?;
    let args = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(v) if v.is_object() => v.clone(),
        Some(_) => {
            return Err(DispatchError::InvalidParams(
                "tools/call arguments must be an object".into(),
            ));
        }
    };

    match run_tool(name, &args, ctx) {
        Ok(payload) => Ok(tool_text(payload, false)),
        Err(err) if name_unknown(name) => Err(DispatchError::InvalidParams(err)),
        Err(err) => Ok(tool_text(json!({ "error": err }), true)),
    }
}

fn name_unknown(name: &str) -> bool {
    tool_defs()
        .iter()
        .all(|t| t.get("name").and_then(Value::as_str) != Some(name))
}

fn run_tool(name: &str, args: &Value, ctx: &McpContext) -> Result<Value, String> {
    match name {
        "cam_index" => {
            let project = project_from(args, ctx)?;
            to_json(index_project(&project).map_err(err_str)?)
        }
        "cam_ls" => {
            let project = project_from(args, ctx)?;
            to_json(ls(&project, arg_str(args, "virt_path")).map_err(err_str)?)
        }
        "cam_read" => {
            let project = project_from(args, ctx)?;
            let virt_path = required_str(args, "virt_path")?;
            let full = arg_bool(args, "full").unwrap_or(false);
            to_json(read(&project, virt_path, full).map_err(err_str)?)
        }
        "cam_ref" => {
            let project = project_from(args, ctx)?;
            let symbol = required_str(args, "symbol")?;
            let dir = parse_ref_dir(arg_str(args, "dir"))?;
            to_json(refs(&project, symbol, dir).map_err(err_str)?)
        }
        "cam_recall" => {
            let project = project_from(args, ctx)?;
            let query = required_str(args, "query")?;
            let opts = RecallOptions {
                limit: arg_usize(args, "limit").unwrap_or(3),
                fusion: parse_fusion(arg_str(args, "fusion"))?,
                expand: arg_bool(args, "expand").unwrap_or(true),
            };
            let embedder = embedder_from(args)?;
            to_json(
                crate::memory::recall(&project, embedder.as_ref(), query, opts).map_err(err_str)?,
            )
        }
        "cam_add" => {
            let project = project_from(args, ctx)?;
            let summary = required_str(args, "summary")?;
            let body = required_str(args, "body")?;
            let parent = arg_str(args, "parent");
            let embedder = embedder_from(args)?;
            to_json(
                add_solution(&project, embedder.as_ref(), summary, body, parent)
                    .map_err(err_str)?,
            )
        }
        "cam_mem_tree" => {
            let project = project_from(args, ctx)?;
            to_json(solution_tree(&project).map_err(err_str)?)
        }
        "cam_mem_show" => {
            let project = project_from(args, ctx)?;
            let id = required_str(args, "id")?;
            to_json(show_solution(&project, id).map_err(err_str)?)
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn project_from(args: &Value, ctx: &McpContext) -> Result<Project, String> {
    resolve_project_from_strings(arg_str(args, "path"), ctx.default_path.as_deref()).map_err(err_str)
}

fn embedder_from(args: &Value) -> Result<Box<dyn Embedder>, String> {
    load_embedder(arg_bool(args, "hash_embed").unwrap_or(false)).map_err(err_str)
}

fn to_json(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err_str)
}

fn tool_text(payload: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

fn path_prop() -> Value {
    json!({ "type": "string", "description": "Project root. Defaults to CAM_PROJECT, then the server --project, then .cam / .git walk-up from the server cwd, else the server cwd." })
}

fn tool_defs() -> Vec<Value> {
    vec![
        tool(
            "cam_index",
            "Parse the project with tree-sitter into .cam/cam.db. Equivalent CLI: cam index",
            json!({
                "type": "object",
                "properties": { "path": path_prop() },
                "additionalProperties": false
            }),
            false,
            true,
        ),
        tool(
            "cam_ls",
            "List indexed directories, files, or symbols as a virtual filesystem. Needs an indexed graph; call cam_index first if it reports the graph is not indexed. Equivalent CLI: cam ls [virt_path]",
            json!({
                "type": "object",
                "properties": {
                    "virt_path": { "type": "string", "description": "Virtual path such as src/ or src/main.rs. Omit for repo root." },
                    "path": path_prop()
                },
                "additionalProperties": false
            }),
            true,
            true,
        ),
        tool(
            "cam_read",
            "Read a file outline or a symbol body. Prefer symbol paths (src/main.rs/main) over --full. Needs an indexed graph; call cam_index first if it reports the graph is not indexed. Equivalent CLI: cam read <virt_path> [--full]",
            json!({
                "type": "object",
                "properties": {
                    "virt_path": { "type": "string", "description": "File (src/main.rs) or symbol (src/main.rs/main)." },
                    "full": { "type": "boolean", "description": "Read the whole file instead of the outline.", "default": false },
                    "path": path_prop()
                },
                "required": ["virt_path"],
                "additionalProperties": false
            }),
            true,
            true,
        ),
        tool(
            "cam_ref",
            "One-hop callers (in) or callees (out). Needs an indexed graph; call cam_index first if it reports the graph is not indexed. Equivalent CLI: cam ref <symbol> --dir in|out",
            json!({
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name, node id, or virtual path like src/lib.rs/add." },
                    "dir": { "type": "string", "enum": ["in", "out"], "description": "in = callers, out = callees." },
                    "path": path_prop()
                },
                "required": ["symbol", "dir"],
                "additionalProperties": false
            }),
            true,
            true,
        ),
        tool(
            "cam_recall",
            "Hybrid recall (vector + BM25 fused with RRF by default) plus Ebbinghaus retention. Also pulls in the newest revision of whatever matched, so `latest` is the current answer. A successful hit refreshes retention. Equivalent CLI: cam recall \"<query>\" [--limit N] [--fusion rrf|sum] [--no-expand]",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "One-sentence question, usually in the user's language." },
                    "limit": { "type": "integer", "minimum": 1, "default": 3 },
                    "fusion": { "type": "string", "enum": ["rrf", "sum"], "default": "rrf", "description": "Score fusion: rrf (default) or min-max sum." },
                    "expand": { "type": "boolean", "default": true, "description": "Pull in the newest revision of each match, even when it matches neither path. Set false to score the raw fused list." },
                    "path": path_prop(),
                    "hash_embed": { "type": "boolean", "description": "Use the test hash embedder instead of model2vec." }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            false,
            true,
        ),
        tool(
            "cam_add",
            "Store a solution (summary + full body). Optionally hang it under --parent. Equivalent CLI: cam add --summary \"...\" [--parent ID] --body \"...\" (or --file / stdin)",
            json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "One-line title stored on the solution tree." },
                    "body": { "type": "string", "description": "Full write-up. Required." },
                    "parent": { "type": "string", "description": "Parent solution id when this updates an older node." },
                    "path": path_prop(),
                    "hash_embed": { "type": "boolean", "description": "Use the test hash embedder instead of model2vec." }
                },
                "required": ["summary", "body"],
                "additionalProperties": false
            }),
            false,
            false,
        ),
        tool(
            "cam_mem_tree",
            "Print the solution tree. Equivalent CLI: cam mem tree",
            json!({
                "type": "object",
                "properties": { "path": path_prop() },
                "additionalProperties": false
            }),
            true,
            true,
        ),
        tool(
            "cam_mem_show",
            "Show one memory by id. Equivalent CLI: cam mem show <id>",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Solution id from recall or cam_mem_tree." },
                    "path": path_prop()
                },
                "required": ["id"],
                "additionalProperties": false
            }),
            true,
            true,
        ),
    ]
}

fn tool(
    name: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
    idempotent: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": idempotent,
            "openWorldHint": false
        }
    })
}

fn parse_ref_dir(value: Option<&str>) -> Result<RefDir, String> {
    match value.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "in" => Ok(RefDir::In),
        "out" => Ok(RefDir::Out),
        other => Err(format!("dir must be in or out, got {other:?}")),
    }
}

fn parse_fusion(value: Option<&str>) -> Result<Fusion, String> {
    match value.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "" | "rrf" => Ok(Fusion::Rrf),
        "sum" => Ok(Fusion::Sum),
        other => Err(format!("fusion must be rrf or sum, got {other:?}")),
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    arg_str(args, key).ok_or_else(|| format!("{key} is required"))
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn arg_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn arg_usize(args: &Value, key: &str) -> Option<usize> {
    let value = args.get(key)?;
    if let Some(n) = value.as_u64() {
        return Some(n as usize);
    }
    if let Some(n) = value.as_i64().filter(|n| *n > 0) {
        return Some(n as usize);
    }
    value
        .as_str()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
}

fn err_str(err: impl std::fmt::Display) -> String {
    err.to_string()
}

fn ok_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn invalid_request(id: Value) -> JsonRpcResponse {
    error_response(id, -32600, "Invalid Request")
}

fn error_response(id: Value, code: i32, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_negotiates_known_version() {
        let resp = handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
            &McpContext::default(),
        )
        .unwrap();
        let value = serde_json::to_value(resp).unwrap();
        assert_eq!(value["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(value["result"]["serverInfo"]["name"], "cam");
        assert!(value["result"]["instructions"].as_str().unwrap().contains("cam_recall"));
    }

    #[test]
    fn notification_has_no_response() {
        let resp = handle_message(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &McpContext::default(),
        );
        assert!(resp.is_none());
    }

    #[test]
    fn tools_list_contains_cam_tools() {
        let resp = handle_message(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            &McpContext::default(),
        )
        .unwrap();
        let value = serde_json::to_value(resp).unwrap();
        let names: Vec<_> = value["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"cam_recall"));
        assert!(names.contains(&"cam_add"));
        assert!(names.contains(&"cam_read"));
        assert!(!names.contains(&"cam_init"));
    }

    #[test]
    fn project_tool_metadata_is_compatible() {
        assert!(INSTRUCTIONS.contains("--project"));
        assert!(!INSTRUCTIONS.contains("--path"));
        for def in tool_defs() {
            assert_eq!(def["inputSchema"]["properties"]["path"]["type"], "string");
            assert!(!def.to_string().contains("--path"));
            match def["name"].as_str().unwrap() {
                "cam_index" | "cam_recall" => {
                    assert_eq!(def["annotations"]["readOnlyHint"], false);
                    assert_eq!(def["annotations"]["idempotentHint"], true);
                }
                "cam_add" => assert_eq!(def["annotations"]["idempotentHint"], false),
                _ => {}
            }
        }
    }

    #[test]
    fn unknown_method_is_32601() {
        let resp = handle_message(
            r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#,
            &McpContext::default(),
        )
        .unwrap();
        let value = serde_json::to_value(resp).unwrap();
        assert_eq!(value["error"]["code"], -32601);
    }
}
