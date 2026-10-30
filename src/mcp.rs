use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

use crate::code::{find, graph_counts, index_project, ls, read, refs, RefDir, SymbolHints};
use crate::memory::{
    add_memories, add_solution_with_links, show_solution, solution_tree, Embedder, NewMemory,
    RecallOptions,
};
use crate::ops::{load_embedder, resolve_project_from_strings};
use crate::project::Project;

const SERVER_NAME: &str = "cam";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const FALLBACK_PROTOCOL_VERSION: &str = "2025-03-26";

const INSTRUCTIONS: &str = r##"cam is local code-graph + solution memory for this repo (SQLite under .cam/).

Main agent: call these MCP tools. Do not shell out to `cam` unless a tool is missing or fails.

Workflow:
1. cam_recall first, with the question written in English. Reuse a hit when it is latest and not needs_update. A hit with body_truncated (only with snippets) carries a snippet: cam_mem_show it before relying on details.
2. Before reading code, make sure the graph is indexed: if cam_ls / cam_find / cam_read / cam_ref report an empty or not-indexed graph, call cam_index once for that project, then retry. cam_index rebuilds the whole graph, so use `cam sync` (CLI) for later edits instead of indexing again.
3. On miss / stale / needs_update: cam_find (part of a symbol name) or cam_ls → cam_read (prefer symbol paths) → cam_ref. cam_find matches symbol names only; use your own grep for text inside function bodies.
4. After you solve it, store what you learned with cam_add. One memory is one fact, or the solution path for one problem (symptom, cause, change, and how to verify). If two pieces can be asked about separately, or can become outdated separately, store them as two memories. A body holds only the detail that one memory needs.
   Put several memories from the same solution in one cam_add call as `memories`. Inside that array, parent, supersedes, and links may use #0, #1, ... for other items. Recall does not walk links. Example: {"memories":[{"summary":"The shell tool truncates output at 1000 lines","body":"formatLargeTextForModel keeps the last 1000 lines."},{"summary":"Images use a separate size cap","body":"Images over 5 MB base64 are replaced with a not-sent note.","links":["#0"]}]}. A later cam_add can link by the returned id. Summary is English; the body may contain non-Latin characters. supersedes replaces an older revision; parent only sets the structural tree parent.

Subagents usually have no MCP. Instruct them to run the equivalent CLI with --json in the project directory (see each tool description), including `cam index` once if the graph is not built. `cam add` stores one memory per invocation; connect it to an earlier one with `--link <id>`.

Virtual paths: `src/main.rs` is a file; `src/main.rs/main` is a symbol in that file.
Ambiguity: when a bare name matches several definitions, cam_ref / cam_read return status `ambiguous` with `candidates` (not an error). Re-call with a candidate `id`, or pass `file` / `kind` / `scope` hints to cam_ref. In a folder holding several repos, cam_ls lists each nested repo with kind `project`; use that path as `scope`.
Ignore rules: `.gitignore` and `.camignore` (same syntax) at any depth. A nested directory with its own `.git` (clone, submodule, or linked worktree) is indexed into that directory's `.cam/cam.db`; the parent database stores the catalog and queries fan out, so opening the nested directory reuses its graph.
Project resolution: tool argument `path`, else CAM_PROJECT, else server `--project`, else .cam / .git walk-up from the server cwd, else the server cwd. `.cam/` and the database are created automatically on the first tool call."##;

const RESTART_NOTICE: &str = "cam 已更新，请重启 MCP 服务";

fn server_version() -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("CAM_GIT_HASH"))
}

#[derive(Debug, Clone, Default)]
pub struct McpContext {
    pub default_path: Option<PathBuf>,
    /// Executable stamp taken when the process started. Tool results mention a
    /// restart when the file at that path later has a different mtime.
    binary: Option<BinaryStamp>,
}

#[derive(Debug, Clone)]
struct BinaryStamp {
    path: PathBuf,
    mtime: SystemTime,
}

impl BinaryStamp {
    fn capture() -> Option<Self> {
        let path = std::env::current_exe().ok()?;
        let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
        Some(Self { path, mtime })
    }

    fn changed(&self) -> bool {
        match std::fs::metadata(&self.path).and_then(|meta| meta.modified()) {
            Ok(mtime) => mtime != self.mtime,
            Err(_) => false,
        }
    }
}

pub fn serve_stdio(default_path: Option<&Path>) -> Result<()> {
    let ctx = McpContext {
        default_path: default_path.map(Path::to_path_buf),
        binary: BinaryStamp::capture(),
    };
    eprintln!("cam mcp {} ready (stdio)", env!("CARGO_PKG_VERSION"));

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
        "prompts/list" => Ok(json!({ "prompts": prompt_defs() })),
        "prompts/get" => get_prompt(params.unwrap_or(&Value::Null)),
        "resources/list" => Ok(json!({ "resources": resource_defs() })),
        "resources/read" => read_resource(params.unwrap_or(&Value::Null), ctx),
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
            "tools": { "listChanged": false },
            "prompts": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false }
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": server_version()
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
        Ok(payload) => Ok(tool_text(payload, false, ctx)),
        Err(err) if name_unknown(name) => Err(DispatchError::InvalidParams(err)),
        Err(err) => Ok(tool_text(json!({ "error": err }), true, ctx)),
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
            let hints = SymbolHints {
                file: arg_str(args, "file"),
                kind: arg_str(args, "kind"),
                scope: arg_str(args, "scope"),
            };
            to_json(refs(&project, symbol, dir, hints).map_err(err_str)?)
        }
        "cam_find" => {
            let project = project_from(args, ctx)?;
            let query = required_str(args, "query")?;
            let hints = SymbolHints {
                file: arg_str(args, "file"),
                kind: arg_str(args, "kind"),
                scope: arg_str(args, "scope"),
            };
            let limit = arg_usize(args, "limit").unwrap_or(20);
            to_json(find(&project, query, hints, limit).map_err(err_str)?)
        }
        "cam_recall" => {
            let project = project_from(args, ctx)?;
            let query = required_str(args, "query")?;
            let opts = RecallOptions {
                limit: arg_usize(args, "limit").unwrap_or(3),
                expand: arg_bool(args, "expand").unwrap_or(true),
                include_superseded: arg_bool(args, "include_superseded").unwrap_or(false),
                snippets: arg_bool(args, "snippets").unwrap_or(false),
            };
            let embedder = embedder_from(args)?;
            to_json(
                crate::memory::recall(&project, embedder.as_ref(), query, opts).map_err(err_str)?,
            )
        }
        "cam_add" => {
            let project = project_from(args, ctx)?;
            let embedder = embedder_from(args)?;
            if let Some(memories) = args.get("memories").filter(|value| !value.is_null()) {
                if ["summary", "body", "parent", "supersedes", "links"]
                    .into_iter()
                    .any(|key| field_present(args, key))
                {
                    return Err("when memories is set, put summary, body, parent, supersedes, and links on each item".into());
                }
                let memories = parse_new_memories(memories)?;
                to_json(add_memories(&project, embedder.as_ref(), &memories).map_err(err_str)?)
            } else {
                let summary = required_str(args, "summary")?;
                let body = required_str(args, "body")?;
                let parent = arg_str(args, "parent");
                let supersedes = arg_str(args, "supersedes");
                let links = arg_string_list(args, "links")?;
                to_json(
                    add_solution_with_links(
                        &project,
                        embedder.as_ref(),
                        summary,
                        body,
                        parent,
                        supersedes,
                        &links,
                    )
                    .map_err(err_str)?,
                )
            }
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
    resolve_project_from_strings(arg_str(args, "path"), ctx.default_path.as_deref())
        .map_err(err_str)
}

fn embedder_from(args: &Value) -> Result<Box<dyn Embedder>, String> {
    load_embedder(arg_bool(args, "hash_embed").unwrap_or(false)).map_err(err_str)
}

fn to_json(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err_str)
}

/// Compact JSON: tool text goes straight into the agent's context, where
/// pretty-print indentation is pure token cost.
fn tool_text(payload: Value, is_error: bool, ctx: &McpContext) -> Value {
    let mut content = vec![json!({ "type": "text", "text": payload.to_string() })];
    if ctx.binary.as_ref().is_some_and(|stamp| stamp.changed()) {
        content.push(json!({ "type": "text", "text": RESTART_NOTICE }));
    }
    json!({
        "content": content,
        "isError": is_error
    })
}

fn field_present(args: &Value, key: &str) -> bool {
    match args.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(value)) => !value.trim().is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(_) => true,
    }
}

fn parse_new_memories(value: &Value) -> Result<Vec<NewMemory>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "memories must be an array".to_string())?;
    if items.is_empty() {
        return Err("memories must contain at least one memory".to_string());
    }
    let mut memories = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| format!("memories[{index}] must be an object"))?;
        for key in obj.keys() {
            if !matches!(
                key.as_str(),
                "summary" | "body" | "parent" | "supersedes" | "links"
            ) {
                return Err(format!("memories[{index}] has unknown field {key}"));
            }
        }
        let summary = obj
            .get("summary")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .ok_or_else(|| format!("memories[{index}].summary is required"))?;
        let body = obj
            .get("body")
            .and_then(Value::as_str)
            .filter(|body| !body.trim().is_empty())
            .ok_or_else(|| format!("memories[{index}].body is required"))?;
        memories.push(NewMemory {
            summary: summary.to_string(),
            body: body.to_string(),
            parent: optional_item_str(obj, index, "parent")?,
            supersedes: optional_item_str(obj, index, "supersedes")?,
            links: item_string_list(obj, index, "links")?,
        });
    }
    Ok(memories)
}

fn optional_item_str(
    obj: &serde_json::Map<String, Value>,
    index: usize,
    key: &str,
) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.to_string()))
            }
        }
        Some(_) => Err(format!("memories[{index}].{key} must be a string")),
    }
}

fn item_string_list(
    obj: &serde_json::Map<String, Value>,
    index: usize,
    key: &str,
) -> Result<Vec<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Some(value) = item.as_str() else {
                    return Err(format!(
                        "memories[{index}].{key} must be an array of strings"
                    ));
                };
                let value = value.trim();
                if !value.is_empty() {
                    out.push(value.to_string());
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!(
            "memories[{index}].{key} must be an array of strings"
        )),
    }
}

fn path_prop() -> Value {
    json!({ "type": "string", "description": "Project root. Defaults to CAM_PROJECT, then the server --project, then .cam / .git walk-up from the server cwd, else the server cwd." })
}

fn tool_defs() -> Vec<Value> {
    vec![
        tool(
            "cam_index",
            "Parse the project with tree-sitter into .cam/cam.db. Nested directories with their own .git are indexed into that directory's .cam/cam.db; this database keeps a catalog of them. Equivalent CLI: cam index",
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
            "List indexed directories, files, or symbols as a virtual filesystem. Directories that are git checkouts of their own are listed with kind `project`; pass such a path as `scope` to cam_ref in multi-repo workspaces. Needs an indexed graph; call cam_index first if it reports the graph is not indexed. Equivalent CLI: cam ls [virt_path]",
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
            "Read a file outline or a symbol body. Prefer symbol paths (src/main.rs/main) over --full. If the name occurs more than once in the file the result has status `ambiguous` with `candidates`; re-call with a candidate `id` as virt_path. Needs an indexed graph; call cam_index first if it reports the graph is not indexed. Equivalent CLI: cam read <virt_path> [--full]",
            json!({
                "type": "object",
                "properties": {
                    "virt_path": { "type": "string", "description": "File (src/main.rs), symbol (src/main.rs/main), or a node id from an ambiguous answer." },
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
            "One-hop callers (in) or callees (out). A bare name that matches several definitions returns status `ambiguous` with ranked `candidates` instead of refs; re-call with a candidate `id` as symbol, or narrow with `file` / `kind` / `scope`. Needs an indexed graph; call cam_index first if it reports the graph is not indexed. Equivalent CLI: cam ref <symbol> --dir in|out [--file SUBSTR] [--kind KIND] [--scope DIR]",
            json!({
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name, node id, or virtual path like src/lib.rs/add." },
                    "dir": { "type": "string", "enum": ["in", "out"], "description": "in = callers, out = callees." },
                    "file": { "type": "string", "description": "Disambiguate: case-insensitive substring of the defining file path, e.g. core/src/config.ts." },
                    "kind": { "type": "string", "description": "Disambiguate: node kind (function, method, struct, class, trait, enum, enum_variant, type_alias)." },
                    "scope": { "type": "string", "description": "Restrict resolution and refs to this sub-directory, typically a `project` entry from cam_ls (e.g. codely-cli)." },
                    "path": path_prop()
                },
                "required": ["symbol", "dir"],
                "additionalProperties": false
            }),
            true,
            true,
        ),
        tool(
            "cam_find",
            "Find symbols whose name contains a substring (case-insensitive). Exact names rank first, then prefixes, then other matches. Each hit has a virtual `path` and an `id` to pass to cam_read or cam_ref. Matches names only, not text inside bodies. Needs an indexed graph; call cam_index first if it reports the graph is not indexed. Equivalent CLI: cam find <query> [--kind KIND] [--file SUBSTR] [--scope DIR] [--limit N]",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Part of a symbol name, e.g. recall or parse_ref." },
                    "kind": { "type": "string", "description": "Keep only this node kind (function, method, struct, class, trait, enum, enum_variant, type_alias)." },
                    "file": { "type": "string", "description": "Keep only symbols whose file path contains this (case-insensitive)." },
                    "scope": { "type": "string", "description": "Keep only symbols under this sub-directory, typically a `project` entry from cam_ls." },
                    "limit": { "type": "integer", "minimum": 1, "default": 20 },
                    "path": path_prop()
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            true,
            true,
        ),
        tool(
            "cam_recall",
            "Hybrid recall (vector + BM25 fused with RRF), scaled slightly by Ebbinghaus retention. The query must be written in English. `score` near 2 means both paths matched, near 1 only one. Every hit carries its full body unless `snippets` is set; then long bodies below the top hit come back clipped with `body_truncated: true`, and cam_mem_show returns the full text. By default it pulls in the newest revision and hides superseded revisions. A successful returned match refreshes retention. Equivalent CLI: cam recall \"<query>\" [--limit N] [--no-expand] [--include-superseded] [--snippets]",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "One-sentence question written in English. Non-Latin scripts are rejected." },
                    "limit": { "type": "integer", "minimum": 1, "default": 3 },
                    "expand": { "type": "boolean", "default": true, "description": "Pull in the newest revision of each match, even when it matches neither path. Set false to score the raw fused list." },
                    "include_superseded": { "type": "boolean", "default": false, "description": "Return older revisions as well as the latest one for history or comparison queries." },
                    "snippets": { "type": "boolean", "default": false, "description": "Clip long bodies below the top hit to snippets marked body_truncated; read one in full with cam_mem_show." },
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
            "Store one memory, or several memories from the same solution. Each memory is one fact, or the solution path for one problem (symptom, cause, change, and how to verify). If two pieces can be asked about separately, or can become outdated separately, store them as separate memories; a body holds only the detail one memory needs. Pass one memory as summary and body. Pass several as memories, and use #0, #1, ... inside that array for parent, supersedes, and links. Summary must be English; the body supports non-Latin characters. links connects related memories (recall does not walk them). parent is structural hierarchy. supersedes replaces an older revision. Equivalent CLI, one memory per invocation: cam add --summary \"...\" [--parent ID] [--supersedes ID] [--link ID]... --body \"...\" (or --file / stdin)",
            json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "One-line English title for this one fact or solution path. Must be written in English." },
                    "body": { "type": "string", "description": "Detail this one fact or solution path needs, and nothing else. Supports non-Latin characters." },
                    "parent": { "type": "string", "description": "Structural parent memory id, or #0, #1, ... when this item is inside memories. This relation may branch and does not affect latest." },
                    "supersedes": { "type": "string", "description": "Older revision replaced by this memory. An existing id, or #0, #1, ... inside memories. Omit for a structural child or independent memory." },
                    "links": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Related memory ids. Inside memories, #0, #1, ... refer to other items in that array."
                    },
                    "memories": {
                        "type": "array",
                        "description": "Several memories from the same solution, stored in one call. Each item is one fact or one solution path. parent, supersedes, and links on an item may use #0, #1, ... to refer to other items in this array. Do not also pass top-level summary or body.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "summary": { "type": "string", "description": "One-line English title for this one fact or solution path." },
                                "body": { "type": "string", "description": "Detail this one fact or solution path needs, and nothing else. Supports non-Latin characters." },
                                "parent": { "type": "string", "description": "Structural parent: an existing memory id, or #0, #1, ... in this array." },
                                "supersedes": { "type": "string", "description": "Older revision this memory replaces: an existing id, or #0, #1, ... in this array." },
                                "links": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Related memory ids, or #0, #1, ... in this array."
                                }
                            },
                            "required": ["summary", "body"],
                            "additionalProperties": false
                        }
                    },
                    "path": path_prop(),
                    "hash_embed": { "type": "boolean", "description": "Use the test hash embedder instead of model2vec." }
                },
                "oneOf": [
                    { "required": ["summary", "body"] },
                    { "required": ["memories"] }
                ],
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

fn arg_string_list(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => Ok(s
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()),
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                let Some(s) = item.as_str() else {
                    return Err(format!("{key} items must be strings"));
                };
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
            Ok(out)
        }
        _ => Err(format!("{key} must be an array of strings")),
    }
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

fn prompt_defs() -> Vec<Value> {
    vec![
        json!({
            "name": "explore_code",
            "description": "Look up a project question in memory, then read the code graph.",
            "arguments": [{
                "name": "question",
                "description": "One-sentence question written in English.",
                "required": true
            }]
        }),
        json!({
            "name": "remember_solution",
            "description": "Store what was just learned as one memory per fact or problem solution path.",
            "arguments": [
                {
                    "name": "summary",
                    "description": "One-line English title for one fact or solution path. Split the text first if it contains more than one.",
                    "required": true
                },
                {
                    "name": "body",
                    "description": "Detail for one fact or solution path. If it contains several, split them into a cam_add memories array.",
                    "required": true
                }
            ]
        }),
    ]
}

fn get_prompt(params: &Value) -> Result<Value, DispatchError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::InvalidParams("prompts/get requires params.name".into()))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let text = match name {
        "explore_code" => {
            let question = required_str(&args, "question").map_err(DispatchError::InvalidParams)?;
            format!(
                "Question: {question}\n\n\
                 Use the cam MCP tools in this order:\n\
                 1. Call cam_recall with this question in English. Reuse a hit when it is latest and not needs_update.\n\
                 2. If cam_ls, cam_find, cam_read, or cam_ref says the graph is not indexed, call cam_index once, then retry.\n\
                 3. On a miss, locate symbols with cam_find (part of a name) or cam_ls, then cam_read (prefer a symbol path), then cam_ref."
            )
        }
        "remember_solution" => {
            let summary = required_str(&args, "summary").map_err(DispatchError::InvalidParams)?;
            let body = required_str(&args, "body").map_err(DispatchError::InvalidParams)?;
            format!(
                "Store this as cam memory. One memory is one fact, or the solution path for one problem (symptom, cause, change, and how to verify). If the text below contains more than one, call cam_add once with a memories array and point parent, supersedes, and links at other items with #0, #1, .... Otherwise call cam_add with this summary and body. Summary must be English; the body supports non-Latin characters.\n\n\
                 summary: {summary}\n\n\
                 body:\n{body}"
            )
        }
        other => {
            return Err(DispatchError::InvalidParams(format!(
                "Unknown prompt: {other}"
            )))
        }
    };
    Ok(json!({
        "description": name,
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": text }
        }]
    }))
}

fn resource_defs() -> Vec<Value> {
    vec![
        json!({
            "uri": "cam://status",
            "name": "status",
            "description": "Whether the code graph database exists, with file, symbol, and memory counts.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "cam://memory/tree",
            "name": "memory-tree",
            "description": "Solution memory tree for the resolved project.",
            "mimeType": "application/json"
        }),
    ]
}

fn read_resource(params: &Value, ctx: &McpContext) -> Result<Value, DispatchError> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::InvalidParams("resources/read requires params.uri".into()))?;
    let project = project_for_resource(ctx).map_err(DispatchError::InvalidParams)?;
    let payload = match uri {
        "cam://status" => project_status(&project).map_err(DispatchError::InvalidParams)?,
        "cam://memory/tree" => {
            memory_tree_resource(&project).map_err(DispatchError::InvalidParams)?
        }
        other => {
            return Err(DispatchError::InvalidParams(format!(
                "Unknown resource: {other}"
            )))
        }
    };
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|err| DispatchError::InvalidParams(err.to_string()))?;
    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": text
        }]
    }))
}

fn project_for_resource(ctx: &McpContext) -> Result<Project, String> {
    if let Ok(value) = std::env::var("CAM_PROJECT") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Project::resolve(Some(Path::new(trimmed))).map_err(err_str);
        }
    }
    Project::resolve(ctx.default_path.as_deref()).map_err(err_str)
}

fn project_status(project: &Project) -> Result<Value, String> {
    let db = project.db_path();
    if !db.is_file() {
        return Ok(json!({
            "indexed": false,
            "db": db.display().to_string(),
            "files": 0,
            "symbols": 0,
            "memories": 0
        }));
    }
    let (files, symbols, _) = graph_counts(project).map_err(err_str)?;
    let conn =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(err_str)?;
    Ok(json!({
        "indexed": true,
        "db": db.display().to_string(),
        "files": files,
        "symbols": symbols,
        "memories": sql_count(&conn, "SELECT COUNT(*) FROM solutions")?
    }))
}

fn memory_tree_resource(project: &Project) -> Result<Value, String> {
    if !project.db_path().is_file() {
        return Ok(json!({ "nodes": [] }));
    }
    let nodes = solution_tree(project).map_err(err_str)?;
    to_json(json!({ "nodes": nodes }))
}

fn sql_count(conn: &rusqlite::Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0)).map_err(err_str)
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
        let hash = env!("CAM_GIT_HASH");
        assert!(
            hash == "unknown" || (hash.len() >= 12 && hash.chars().all(|c| c.is_ascii_hexdigit())),
            "{hash}"
        );
        assert_eq!(
            value["result"]["serverInfo"]["version"],
            format!("{}+{hash}", env!("CARGO_PKG_VERSION"))
        );
        assert!(value["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("cam_recall"));
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
    fn memory_tools_expose_separate_structure_and_revision_options() {
        let defs = tool_defs();
        let add = defs.iter().find(|tool| tool["name"] == "cam_add").unwrap();
        assert_eq!(add["inputSchema"]["properties"]["parent"]["type"], "string");
        assert_eq!(
            add["inputSchema"]["properties"]["supersedes"]["type"],
            "string"
        );
        assert_eq!(add["inputSchema"]["properties"]["links"]["type"], "array");
        assert_eq!(
            add["inputSchema"]["properties"]["memories"]["type"],
            "array"
        );
        assert!(add["description"]
            .as_str()
            .unwrap()
            .contains("solution path"));
        assert!(!add.to_string().contains("Full write-up"));
        assert!(INSTRUCTIONS.contains("#0"));
        assert!(INSTRUCTIONS.contains("separately"));
        let recall = defs
            .iter()
            .find(|tool| tool["name"] == "cam_recall")
            .unwrap();
        assert_eq!(
            recall["inputSchema"]["properties"]["include_superseded"]["default"],
            false
        );
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
    fn prompts_and_resources_are_listed() {
        let prompts = handle_message(
            r#"{"jsonrpc":"2.0","id":4,"method":"prompts/list"}"#,
            &McpContext::default(),
        )
        .unwrap();
        let prompts = serde_json::to_value(prompts).unwrap();
        let names: Vec<_> = prompts["result"]["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["name"].as_str())
            .collect();
        assert_eq!(names, ["explore_code", "remember_solution"]);

        let resources = handle_message(
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#,
            &McpContext::default(),
        )
        .unwrap();
        let resources = serde_json::to_value(resources).unwrap();
        let uris: Vec<_> = resources["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["uri"].as_str())
            .collect();
        assert_eq!(uris, ["cam://status", "cam://memory/tree"]);
    }

    #[test]
    fn prompt_get_requires_arguments() {
        let missing = handle_message(
            r#"{"jsonrpc":"2.0","id":6,"method":"prompts/get","params":{"name":"explore_code"}}"#,
            &McpContext::default(),
        )
        .unwrap();
        let missing = serde_json::to_value(missing).unwrap();
        assert_eq!(missing["error"]["code"], -32602);

        let ok = handle_message(
            r#"{"jsonrpc":"2.0","id":7,"method":"prompts/get","params":{"name":"explore_code","arguments":{"question":"where is recall fused"}}}"#,
            &McpContext::default(),
        )
        .unwrap();
        let ok = serde_json::to_value(ok).unwrap();
        let text = ok["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("where is recall fused"));
        assert!(text.contains("cam_recall"));
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

    #[test]
    fn remember_solution_splits_a_body_into_memories() {
        let ok = handle_message(
            r#"{"jsonrpc":"2.0","id":8,"method":"prompts/get","params":{"name":"remember_solution","arguments":{"summary":"Shell truncation","body":"The shell tool keeps 1000 lines."}}}"#,
            &McpContext::default(),
        )
        .unwrap();
        let ok = serde_json::to_value(ok).unwrap();
        let text = ok["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("memories"));
        assert!(text.contains("#0"));
        assert!(text.contains("solution path"));
    }

    #[test]
    fn changed_binary_appends_restart_notice() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("cam");
        std::fs::write(&bin, b"v1").unwrap();
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&bin)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let stamp = BinaryStamp {
            path: bin.clone(),
            mtime: std::fs::metadata(&bin).unwrap().modified().unwrap(),
        };
        assert!(!stamp.changed());
        std::fs::File::options()
            .write(true)
            .open(&bin)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000))
            .unwrap();
        assert!(stamp.changed());

        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let ctx = McpContext {
            default_path: None,
            binary: Some(stamp),
        };
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "cam_mem_tree",
                "arguments": { "path": project }
            }
        });
        let resp = handle_message(&request.to_string(), &ctx).unwrap();
        let value = serde_json::to_value(resp).unwrap();
        let content = value["result"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[1]["text"], RESTART_NOTICE);
        assert_eq!(value["result"]["isError"], false);
        serde_json::from_str::<Value>(content[0]["text"].as_str().unwrap()).unwrap();

        let failed = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "cam_add",
                "arguments": { "path": project, "hash_embed": true }
            }
        });
        let failed = handle_message(&failed.to_string(), &ctx).unwrap();
        let failed = serde_json::to_value(failed).unwrap();
        assert_eq!(failed["result"]["isError"], true);
        assert_eq!(failed["result"]["content"][1]["text"], RESTART_NOTICE);
    }
}
