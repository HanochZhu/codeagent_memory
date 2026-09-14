# cam

[English](README.md) | [中文](README.zh.md)

Local memory CLI for CodeAgents. Index a project into a SQLite code graph, then recall past solutions so an agent does not re-read or re-solve the same problem.

Agents talk to `cam` **over the shell only** — no MCP. Output is compact by default; pass `--json` when you need structured results.

See [DESIGN.md](DESIGN.md) for the design notes (Chinese).

## One-sentence install (paste into an AI)

Drop this whole block into Cursor, Claude Code, Windsurf, GitHub Copilot, Continue, Cline, or JetBrains AI Assistant:

```text
Install cam from https://github.com/HanochZhu/codeagent_memory: require a working Rust toolchain (rustup), run `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`, ensure ~/.cargo/bin (or %USERPROFILE%\.cargo\bin on Windows) is on PATH, then verify with `cam --help`. In this repo run `cam init` and `cam index`. Do not add extra files.
```

| Environment | How to use it |
| --- | --- |
| **Cursor** | Paste into Agent / Chat; it will run the install in the terminal |
| **VS Code** + Copilot / Continue / Cline | Same: paste into Chat / Agent |
| **Claude Code** / **Windsurf** / **Codex** | Paste in the conversation and let it run the shell |
| **JetBrains** (IntelliJ / RustRover / GoLand) | AI Assistant, or run the same `cargo install` in the built-in terminal |
| **Local terminal** | Skip the AI; run the commands in the next section |

Prerequisite: [Rust](https://rustup.rs/) (stable, edition 2021). The first `recall` / `add` downloads `potion-multilingual-128M`; if the model is unavailable, `cam` falls back to a hash embedder.

## Manual install

```bash
cargo install --git https://github.com/HanochZhu/codeagent_memory --locked
cam --help
```

Build from a local clone:

```bash
git clone https://github.com/HanochZhu/codeagent_memory.git
cd codeagent_memory
cargo install --path . --locked
```

## Quick start

From the root of the project you want to remember:

```bash
cam init
cam index
cam ls src/
cam read src/main.rs/main
cam ref main --dir in
cam recall "how does hybrid recall fuse BM25 and vectors"
cam add --summary "..." < notes.md
```

Global flags: `--json`, `--path <project>`. Without `--path`, `cam` walks up for `.cam` or `.git`.

## Agent workflow

`recall` first. If nothing hits, walk the code graph. After you solve it, `add`.

```text
cam init
cam index
cam ls src/
cam read src/memory/recall.rs/fuse_scores
cam ref fuse_scores --dir in
cam recall "how to fuse BM25 and vector recall"
cam add --summary "..." --parent <id>
cam mem tree
cam mem show <id>
```

| Command | What it does |
| --- | --- |
| `cam init` | Create `.cam/` and register the project |
| `cam index` | Parse with tree-sitter into SQLite (`.cam/cam.db`) |
| `cam ls [path]` | List directories / files / symbols |
| `cam read <path>` | File outline or symbol body; `--full` for the whole file |
| `cam ref <symbol> --dir in\|out` | One-hop callers / callees |
| `cam recall "<one sentence>"` | Vector + BM25; scores are min-max normalized then **summed** |
| `cam add --summary "..." [--parent ID]` | Store a solution (body from stdin or `--file`) |
| `cam mem tree` / `cam mem show <id>` | Browse the solution tree |

Virtual paths: `src/main.rs` is a file; `src/main.rs/main` is a symbol in that file.

Languages: Rust, Python, TypeScript, JavaScript, Go.

## Memory rules (for agents)

- Entries older than `stale_days` in `~/.cam/config.toml` (default 30) are marked `stale`
- Ebbinghaus retention `R = exp(-t / S)` is added to the recall score; `R < 0.3` is marked `needs_update`
- Memories are never deleted. When several sit on the same path, the newest is `latest`
- To update, `add` a new node (optionally `--parent`); the old node stays on the tree
- Chinese BM25 is jieba-tokenized before FTS5

## Config

`~/.cam/config.toml`:

```toml
stale_days = 30
```

Vector models are cached under `~/.cam/models/`.
