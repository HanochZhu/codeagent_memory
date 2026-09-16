---
name: cam
description: Use when a repo has a `.cam/` directory, or the user asks whether a problem was solved before, wants to navigate symbols / callers / callees without reading whole files, or asks to run `cam`. Guides agents to reuse cam solution memory and query the code graph instead of grep/glob/Read.
---

# cam — code graph + solution memory

`cam` is a local (SQLite, no API keys) memory for coding agents. It has two halves:

- **Code graph** — every indexed file and symbol as a virtual filesystem, plus one-hop callers / callees.
- **Solution tree** — past write-ups, recalled with vector + BM25 so you do not re-solve the same problem.

Check for a `.cam/` directory at the project root. If the repo is not initialized yet, run `cam init` then `cam index` once.

## Who calls what

| Caller | Interface |
| --- | --- |
| **Main agent** (this session) | MCP tools `cam_recall`, `cam_ls`, `cam_read`, `cam_ref`, `cam_add`, `cam_mem_tree`, `cam_mem_show` |
| **Subagent** (Task / explore / delegated) | `cam --json …` in a shell — subagents usually do not get MCP |
| No MCP wired | Fall back to `cam --json …` |

Prefer the MCP tool when it is available. Do not shell out to `cam` if `cam_*` tools exist.

## Workflow

1. **Recall first.** `cam_recall` (MCP) / `cam recall "<one sentence>"` (CLI). Query in the user's language.
   - Reuse a hit only when it is `latest` and not `needs_update`.
   - `stale` / `needs_update` / no hit → continue.
2. **Walk the graph** on a miss: `ls` → `read` (symbol path) → `ref`.
3. **Write back** after solving: `cam_add` with a short `summary` and the full `body`. Set `parent` to extend an older node instead of duplicating.

Virtual paths: `src/main.rs` is a file; `src/main.rs/main` is a symbol in that file. Prefer symbol reads (`src/foo.rs/bar`) over `full`, and prefer `read`/`ls` over opening whole files.

## Commands

```text
cam init [path]                                  # create .cam/, register the project
cam index [path]                                 # tree-sitter → .cam/cam.db
cam sync [path]                                  # incremental update for changed files
cam ls [virt_path]                               # dirs / files / symbols
cam read <virt_path> [--full]                    # outline, or a symbol body
cam ref <symbol> --dir in|out                    # one-hop callers / callees
cam recall "<query>" [--limit N] [--fusion rrf|sum]
cam add --summary "..." [--parent ID] [--file PATH]   # body: --file or stdin
cam mem tree | cam mem show <id>
cam mcp                                          # stdio MCP server (main agent)
```

Global flags: `--json` for structured output; `--path <project>` when the cwd is not the repo. Project root resolution: `--path` / tool `path` → env `CAM_PROJECT` → walk up for `.cam` / `.git`.

## Memory rules

- Recall score is vector + BM25 fused with **RRF by default** (`--fusion sum` for min-max sum), plus Ebbinghaus retention `R = exp(-t / S)`.
- `R < 0.3` → `needs_update`; older than `stale_days` (default 30) → `stale`.
- A successful recall refreshes retention (`S *= 1.7`).
- Memories are never deleted. Newer nodes on the same path are `latest`; update by adding a child with `parent`.

## Wiring MCP

Main agent hosts register a stdio server: command `cam`, args `["mcp"]` (optional `["--path", "/abs/project", "mcp"]`). Full per-IDE configs: `docs/mcp.md`. Agent prompts: `docs/agents.md`.
