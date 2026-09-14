# cam

[English](README.md) | [中文](README.zh.md)

CodeAgent 的本地记忆 CLI。把项目编成 SQLite 代码图，再按解法召回，避免 Agent 反复读同一段代码、重解同一类问题。

Agent **只走 shell**，不接 MCP。输出默认尽量短；需要结构化结果时加 `--json`。

设计细节见 [DESIGN.md](DESIGN.md)。

## 一句话安装（丢给 AI）

把下面整段贴进 Cursor、Claude Code、Windsurf、GitHub Copilot、Continue、Cline，或 JetBrains AI Assistant：

```text
请从 https://github.com/HanochZhu/codeagent_memory 安装 cam：先确认本机有 rustup/cargo，执行 `cargo install --git https://github.com/HanochZhu/codeagent_memory --locked`，把 ~/.cargo/bin（Windows 为 %USERPROFILE%\.cargo\bin）加入 PATH，用 `cam --help` 验证。然后在当前仓库执行 `cam init` 和 `cam index`。不要额外新建文件。
```

英文版（给英文 Agent 用）见 [README.md](README.md)。

| 环境 | 怎么用这段话 |
| --- | --- |
| **Cursor** | Agent / Chat 里粘贴即可；它会自己开终端装 |
| **VS Code** + Copilot / Continue / Cline | 同样丢给 Chat / Agent |
| **Claude Code** / **Windsurf** / **Codex** | 对话里粘贴，让它跑 shell |
| **JetBrains** (IntelliJ / RustRover / GoLand) | AI Assistant 或内置终端执行同一条 `cargo install` |
| **本机终端** | 不需要 AI，直接跑下一节命令 |

前提：已安装 [Rust](https://rustup.rs/)（建议 stable，edition 2021）。首次 `recall` / `add` 会下载 `potion-multilingual-128M`；模型不可用时自动回退 hash embedder。

## 手动安装

```bash
cargo install --git https://github.com/HanochZhu/codeagent_memory --locked
cam --help
```

从本地克隆编译：

```bash
git clone https://github.com/HanochZhu/codeagent_memory.git
cd codeagent_memory
cargo install --path . --locked
```

## 快速开始

在目标项目根目录：

```bash
cam init
cam index
cam ls src/
cam read src/main.rs/main
cam ref main --dir in
cam recall "how does hybrid recall fuse BM25 and vectors"
cam add --summary "..." < notes.md
```

全局参数：`--json`、`--path <project>`。未指定 `--path` 时向上查找 `.cam` 或 `.git`。

## Agent 怎么用

探索仓库前先 `recall`；没有命中再读代码图；解完再 `add`。

```text
cam init
cam index
cam ls src/
cam read src/memory/recall.rs/fuse_scores
cam ref fuse_scores --dir in
cam recall "如何做 BM25 和向量的多路召回"
cam add --summary "..." --parent <id>
cam mem tree
cam mem show <id>
```

| 命令 | 作用 |
| --- | --- |
| `cam init` | 创建 `.cam/`，登记当前项目 |
| `cam index` | tree-sitter 解析进 SQLite（`.cam/cam.db`） |
| `cam ls [path]` | 列目录 / 文件 / 符号 |
| `cam read <path>` | 文件大纲，或符号源码；`--full` 才整文件 |
| `cam ref <symbol> --dir in\|out` | 一跳 callers / callees |
| `cam recall "<一句话>"` | 向量 + BM25，分数各自归一后**求和** |
| `cam add --summary "..." [--parent ID]` | 写入解法（正文来自 stdin 或 `--file`） |
| `cam mem tree` / `cam mem show <id>` | 浏览记忆树 |

虚拟路径：`src/main.rs` 是文件，`src/main.rs/main` 是该文件里的符号。

语言：Rust、Python、TypeScript、JavaScript、Go。

## 记忆规则（给 Agent）

- 超过 `~/.cam/config.toml` 的 `stale_days`（默认 30）会标 `stale`
- 艾宾浩斯保留率 `R = exp(-t / S)` 会加进召回分；`R < 0.3` 标 `needs_update`
- 记忆不删除。同路径多条并存时标 `latest`，采用更新的那条
- 需要更新时 `add` 新节点（可挂 `--parent`），旧节点留在树上
- 中文 BM25 先 jieba 再进 FTS5

## 配置

`~/.cam/config.toml`：

```toml
stale_days = 30
```

向量模型缓存在 `~/.cam/models/`。
