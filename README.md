<div align="center">

# cam

[English](README.md) · [中文](README.zh.md)

### Give Claude Code, Cursor, Codex, Windsurf, Copilot, and JetBrains a local memory they can query from the shell

**Code graph + solution recall · surgical reads · 100% local · no MCP**

**Kernel written in Rust**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-edition%202021-orange.svg)](https://www.rust-lang.org/)
[![Install](https://img.shields.io/badge/install-cargo--git-blue.svg)](#get-started)

[![Windows](https://img.shields.io/badge/Windows-supported-blue.svg)](#supported-platforms)
[![macOS](https://img.shields.io/badge/macOS-supported-blue.svg)](#supported-platforms)
[![Linux](https://img.shields.io/badge/Linux-supported-blue.svg)](#supported-platforms)

[![Claude Code](https://img.shields.io/badge/Claude_Code-supported-blueviolet.svg)](#supported-agents)
[![Cursor](https://img.shields.io/badge/Cursor-supported-blueviolet.svg)](#supported-agents)
[![Codex](https://img.shields.io/badge/Codex-supported-blueviolet.svg)](#supported-agents)
[![Windsurf](https://img.shields.io/badge/Windsurf-supported-blueviolet.svg)](#supported-agents)
[![GitHub Copilot](https://img.shields.io/badge/GitHub_Copilot-supported-blueviolet.svg)](#supported-agents)
[![Continue](https://img.shields.io/badge/Continue-supported-blueviolet.svg)](#supported-agents)
[![Cline](https://img.shields.io/badge/Cline-supported-blueviolet.svg)](#supported-agents)
[![JetBrains](https://img.shields.io/badge/JetBrains-supported-blueviolet.svg)](#supported-agents)

</div>

## Contents

- [Get Started](#get-started)
- [Why cam?](#why-cam)
- [Key Features](#key-features)
- [How It Works](#how-it-works)
- [Agent Workflow](#agent-workflow)
- [CLI Reference](#cli-reference)
- [Memory Rules](#memory-rules)
- [Configuration](#configuration)
- [Supported Platforms](#supported-platforms)
- [Supported Agents](#supported-agents)
- [Supported Languages](#supported-languages)
- [License](#license)

## Get Started

### 1. Install the CLI

Need Rust first? Install it from [rustup.rs](https://rustup.rs/), then:

```bash
cargo install --git https://github.com/HanochZhu/codeagent_memory --locked
cam --help
```

<details>
<summary><b>Paste this into an AI instead (Cursor, Claude Code, Copilot, …)</b></summary>

```text
Install cam from https://github.com/HanochZhu/codeagent_memory: require a working Rust toolchain (rustup), run `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`, ensure ~/.cargo/bin (or %USERPROFILE%\.cargo\bin on Windows) is on PATH, then verify with `cam --help`. In this repo run `cam init` and `cam index`. Do not add extra files.
```

| Environment | How to use it |
| --- | --- |
| **Cursor** | Paste into Agent / Chat; it runs the install in the terminal |
| **VS Code** + Copilot / Continue / Cline | Same: paste into Chat / Agent |
| **Claude Code** / **Windsurf** / **Codex** | Paste in the conversation and let it run the shell |
| **JetBrains** (IntelliJ / RustRover / GoLand) | AI Assistant, or run the same `cargo install` in the built-in terminal |
| **Local terminal** | Skip the AI; run the command above |

<sub>The installer puts `cam` in `~/.cargo/bin` (Windows: `%USERPROFILE%\.cargo\bin`) but **doesn't change your current shell** — open a new terminal if `cam` is not found.</sub>

</details>

<details>
<summary><b>Build from a local clone</b></summary>

```bash
git clone https://github.com/HanochZhu/codeagent_memory.git
cd codeagent_memory
cargo install --path . --locked
```

</details>

### 2. Initialize the project

```bash
cd your-project
cam init
cam index
```

<sub>`cam init` creates `.cam/` and registers the project. `cam index` parses the tree with tree-sitter into `.cam/cam.db`. These are two steps on purpose: init is cheap, index is the work. Leave `cam watch` running if you want the graph to follow later edits.</sub>

### 3. Ask the graph, then remember the answer

```bash
cam ls src/
cam read src/main.rs/main
cam ref main --dir in
cam recall "how does hybrid recall fuse BM25 and vectors"
cam add --summary "..." < notes.md
```

That's it — your agent talks to `cam` over the **shell only**. No MCP server to wire up. Output is compact by default; pass `--json` when you need structured results.

The first `recall` / `add` downloads `potion-multilingual-128M`. If the model is unavailable, `cam` falls back to a hash embedder.

---

## Why cam?

When an AI agent needs to understand code — or reuse a fix it already found — it discovers structure the slow way: grep, glob, and Read, one file at a time. Next session it does the same work again.

**cam hands the agent two things it can query in one shell call:**

1. A **code graph** — every indexed file and symbol as a virtual filesystem, plus one-hop callers / callees.
2. A **solution tree** — past write-ups, recalled with vector + BM25 so the agent does not re-solve the same problem.

Surgical reads, not a file-by-file search. Memories stay on disk, 100% local.

> Agents talk to `cam` over the shell — no MCP. That is a product choice: one binary, same commands in every IDE, nothing to register in `mcp.json`.

---

## Key Features

| | |
|---|---|
| **Code graph in SQLite** | tree-sitter parse into `.cam/cam.db` — list, read, and hop callers/callees without opening whole files |
| **Live sync** | `cam watch` incrementally updates the graph when source files change |
| **Virtual paths** | `src/main.rs` is a file; `src/main.rs/main` is a symbol in that file |
| **Hybrid recall** | Vector + BM25, each min-max normalized to `[0,1]` then **summed** (not RRF) |
| **Ebbinghaus retention** | `R = exp(-t / S)` is added to the recall score; stale and forgotten entries are flagged, never deleted |
| **Solution tree** | `add` appends a node (optionally `--parent`); the old write-up stays on the tree |
| **Shell-only** | No MCP. Cursor, Claude Code, Copilot, and JetBrains all run the same CLI |
| **100% local** | No API keys. SQLite + an optional on-disk embedding model under `~/.cam/models/` |
| **5 languages** | Rust, Python, TypeScript, JavaScript, Go |

---

## How It Works

```
┌───────────────────────────────────────────────────────────────────┐
│                     Cursor / Claude Code / …                      │
│                                                                   │
│   "Have we solved hybrid BM25 + vector recall before?"            │
│       runs `cam recall "..."` in the shell — no MCP               │
│                                 │                                 │
└─────────────────────────────────┬─────────────────────────────────┘
                                  │
                                  ▼
┌───────────────────────────────────────────────────────────────────┐
│                              cam CLI                              │
│                                                                   │
│  ls / read / ref     →  code graph (tree-sitter)                  │
│  recall / add / mem  →  solution tree (vector + BM25 + FTS5)      │
│                                 │                                 │
│                                 ▼                                 │
│                       local SQLite  (.cam/cam.db)                 │
│          symbols · edges · memories · jieba + FTS5                │
└───────────────────────────────────────────────────────────────────┘
```

1. **Extraction** — tree-sitter walks the project and stores nodes (functions, types) and edges (calls) in SQLite.
2. **Surgical read** — `ls` / `read` / `ref` walk a virtual filesystem. `read` returns an outline or a symbol slice; `--full` is the whole file.
3. **Recall** — `recall` embeds the query, runs BM25 (jieba first for Chinese), min-max normalizes each score, and **sums** them. Retention `R` is added on top.
4. **Write-back** — after the agent solves something, `add` stores summary + body. Same path, newer node wins as `latest`.

Design notes (Chinese): [DESIGN.md](DESIGN.md).

---

## Agent Workflow

`recall` first. If nothing hits, walk the code graph. After you solve it, `add`.

```text
cam init
cam index
cam watch
cam ls src/
cam read src/memory/recall.rs/fuse_scores
cam ref fuse_scores --dir in
cam recall "how to fuse BM25 and vector recall"
cam add --summary "..." --parent <id>
cam mem tree
cam mem show <id>
```

Global flags: `--json`, `--path <project>`. Without `--path`, `cam` walks up for `.cam` or `.git`.

---

## CLI Reference

```bash
cam init [path]                          # Create .cam/ and register the project
cam index [path]                         # Parse the project with tree-sitter into SQLite
cam sync [path]                          # Incrementally update the graph for changed files
cam watch [path]                         # Watch source files and auto-sync after a quiet window
cam ls [virt_path]                       # List directories / files / symbols
cam read <virt_path> [--full]            # File outline or symbol body
cam ref <symbol> --dir in|out            # One-hop callers (in) or callees (out)
cam recall "<query>" [--limit N]         # Hybrid recall: vector + BM25, scores summed
cam add --summary "..." [--parent ID] [--file PATH]   # Store a solution (body: stdin or --file)
cam mem tree                             # Print the solution tree
cam mem show <id>                        # Show one memory
```

| Command | What it does |
| --- | --- |
| `cam init` | Create `.cam/` and register the project |
| `cam index` | Parse with tree-sitter into SQLite (`.cam/cam.db`) |
| `cam sync` | Incrementally update the graph from content hashes |
| `cam watch` | Debounced file watcher that runs `sync` on source changes |
| `cam ls [path]` | List directories / files / symbols |
| `cam read <path>` | File outline or symbol body; `--full` for the whole file |
| `cam ref <symbol> --dir in\|out` | One-hop callers / callees |
| `cam recall "<one sentence>"` | Vector + BM25; scores are min-max normalized then **summed** |
| `cam add --summary "..." [--parent ID]` | Store a solution (body from stdin or `--file`) |
| `cam mem tree` / `cam mem show <id>` | Browse the solution tree |

Virtual paths: `src/main.rs` is a file; `src/main.rs/main` is a symbol in that file.

---

## Memory Rules

- Entries older than `stale_days` in `~/.cam/config.toml` (default 30) are marked `stale`
- Ebbinghaus retention `R = exp(-t / S)` is added to the recall score; `R < 0.3` is marked `needs_update`
- A successful recall (`needs_update = false`) refreshes C0 and multiplies `S` by 1.7
- Memories are never deleted. When several sit on the same path, the newest is `latest`
- To update, `add` a new node (optionally `--parent`); the old node stays on the tree
- Chinese BM25 is jieba-tokenized before FTS5

---

## Configuration

`~/.cam/config.toml`:

```toml
stale_days = 30
```

Vector models are cached under `~/.cam/models/`.

---

## Supported Platforms

`cam` is a Rust binary. `cargo install` builds it for the machine you are on:

| Platform | Install |
|----------|---------|
| Windows | `cargo install --git … --locked` |
| macOS | `cargo install --git … --locked` |
| Linux | `cargo install --git … --locked` |

See [Get Started](#get-started) for the full command.

---

## Supported Agents

`cam` is a shell CLI. Any agent that can run a terminal command can use it — there is nothing to register:

- **Claude Code**
- **Cursor**
- **Codex**
- **Windsurf**
- **GitHub Copilot** (VS Code Chat, Copilot CLI)
- **Continue** / **Cline**
- **JetBrains AI Assistant** (IntelliJ / RustRover / GoLand)

Paste the [one-sentence install](#get-started) into the agent, or run `cargo install` yourself.

---

## Supported Languages

| Language | Extension | What is indexed |
|----------|-----------|-----------------|
| Rust | `.rs` | functions, structs, enums, traits, calls |
| Python | `.py` | functions, classes, calls |
| TypeScript | `.ts`, `.tsx` | functions, methods, classes, calls |
| JavaScript | `.js`, `.jsx` | functions, methods, classes, calls |
| Go | `.go` | functions, methods, structs, calls |

---

## License

MIT

---

<div align="center">

**Made for AI coding agents — Claude Code, Cursor, Codex, Windsurf, Copilot, and JetBrains**

[Report Bug](https://github.com/HanochZhu/codeagent_memory/issues) · [Request Feature](https://github.com/HanochZhu/codeagent_memory/issues)

</div>
