# Agent usage

`cam` has two entry points. Use the one that matches **who is calling**.

| Who | How | Why |
| --- | --- | --- |
| **Main agent** (Cursor / Claude Code / Codex / Windsurf / Copilot session) | MCP tools `cam_*` | Host injects tools; no extra process, structured JSON |
| **Subagent** (Task / explore / background / delegated CLI agent) | `cam --json …` in the project directory | Most hosts do **not** attach MCP to subagents |

Wire MCP first: [docs/mcp.md](mcp.md) (中文: [docs/mcp.zh.md](mcp.zh.md)).

---

## Shared workflow

Same order for both entry points:

1. **Recall first.** If a hit is `latest` and not `needs_update`, reuse it.
2. **Make sure the graph is indexed** before reading code. If `ls` / `read` / `ref` report the graph is not indexed, run `cam_index` / `cam index` once for that project. `cam_index` rebuilds the whole graph; use `cam sync` for later edits.
3. **On miss / stale / needs_update**, walk the code graph: `ls` → `read` (symbol path) → `ref`.
4. **After you solve it**, `add` a node (optionally hang it under `--parent`).

Virtual paths: `src/main.rs` is a file; `src/main.rs/main` is a symbol.

Project root: `--project` / tool argument `path` / env `CAM_PROJECT` / walk up for `.cam` or `.git`.

---

## Main agent (MCP)

After the host loads the `cam` MCP server, call tools. Do **not** shell out to `cam` unless a tool is missing or fails.

| Tool | When |
| --- | --- |
| `cam_recall` | Before exploring. Query in the user's language, one sentence. |
| `cam_ls` | List a directory, file, or the symbols in a file. |
| `cam_read` | File outline, or a symbol body. Prefer `src/foo.rs/bar` over `full=true`. |
| `cam_ref` | One-hop callers (`dir=in`) or callees (`dir=out`). |
| `cam_add` | Persist the write-up. `summary` + full `body`. Set `parent` to update an older node. |
| `cam_mem_tree` / `cam_mem_show` | Browse the solution tree. |
| `cam_init` / `cam_index` | Once per repo (or after large code moves). |

Tool results are JSON text. `cam_recall` refreshes retention on a successful hit (`needs_update = false`). `cam_ls` / `cam_read` / `cam_ref` fail with `not_indexed` when the graph is empty; call `cam_index` once, then retry.

### Paste this into the main agent's rules / AGENTS.md

```text
This repo uses cam (code graph + solution memory).

Main agent: call MCP tools cam_recall / cam_ls / cam_read / cam_ref / cam_add / cam_mem_tree / cam_mem_show. Do not run the cam CLI unless MCP is unavailable.

Workflow: cam_recall first. Before reading code, if cam_ls / cam_read / cam_ref report the graph is not indexed, call cam_index once for that project. On miss or needs_update, walk the graph with cam_ls → cam_read (symbol paths) → cam_ref. After solving, cam_add (summary + full body; parent to extend an older node).

Subagents have no MCP. When you delegate, tell them to run `cam --json` in the project directory (see docs/agents.md).
```

---

## Subagent (CLI)

Subagents launched by Task / explore / `claude -p` / Codex exec / Copilot CLI scripts typically **cannot see MCP**. They must call the binary.

Prefer `--json`. Always pass `--project` if cwd might not be the repo.

```bash
cam --json --project <project> recall "how does hybrid recall fuse BM25 and vectors"
cam --json --project <project> ls src/
cam --json --project <project> read src/memory/recall.rs/fuse_scores
cam --json --project <project> ref fuse_scores --dir in
cam --json --project <project> add --summary "..." --file notes.md
# or:  printf '%s' "$BODY" | cam --json --project <project> add --summary "..."
cam --json --project <project> mem tree
cam --json --project <project> mem show <id>
```

Without `--json`, output is compact text (fine for humans; worse for parsers).

### Paste this into a subagent prompt

```text
You do not have cam MCP tools. Use the cam CLI in the project directory.

cam --json --project <PROJECT> recall "<one sentence>"
cam --json --project <PROJECT> ls [virt_path]
cam --json --project <PROJECT> read <virt_path> [--full]
cam --json --project <PROJECT> ref <symbol> --dir in|out
cam --json --project <PROJECT> add --summary "<one line>" --file <path>
cam --json --project <PROJECT> mem tree
cam --json --project <PROJECT> mem show <id>

Recall first. Before reading code, run `cam index` once if the graph is not built. On miss, ls → read symbol paths → ref. After solving, add. Virtual path: file is src/main.rs; symbol is src/main.rs/main.
```

### Mapping

| MCP (main agent) | CLI (subagent) |
| --- | --- |
| `cam_init` | `cam init` |
| `cam_index` | `cam index` |
| `cam_ls` | `cam ls [virt_path]` |
| `cam_read` | `cam read <virt_path> [--full]` |
| `cam_ref` | `cam ref <symbol> --dir in\|out` |
| `cam_recall` | `cam recall "<query>" [--limit N] [--fusion rrf\|sum]` |
| `cam_add` | `cam add --summary "..." [--parent ID] [--body TEXT \| --file PATH]` |
| `cam_mem_tree` | `cam mem tree` |
| `cam_mem_show` | `cam mem show <id>` |

CLI-only (no MCP tool): `cam sync`, `cam watch`, `cam status`, `cam config get|set stale_days N`.

With `--json`, failures print `{"error":{"code":…,"message":…}}` to stdout and exit non-zero. `--pretty` adds indentation.

---

## When the main agent delegates

The main agent should **pass the CLI cheat sheet**, the project path, and the current `recall` miss (if any). Do not assume the subagent can call `cam_*` tools.

---

## Agent skill

For hosts that support agent skills (opencode, Claude Code), `cam` ships one so the agent knows when to reach for it: [`.opencode/skill/cam/SKILL.md`](../.opencode/skill/cam/SKILL.md). opencode auto-loads it while working in this repo.

Install it globally so every project picks it up:

```bash
# opencode
mkdir -p ~/.config/opencode/skill/cam && cp .opencode/skill/cam/SKILL.md ~/.config/opencode/skill/cam/
# Claude Code
mkdir -p ~/.claude/skills/cam && cp .opencode/skill/cam/SKILL.md ~/.claude/skills/cam/
```

Restart the host after installing.
