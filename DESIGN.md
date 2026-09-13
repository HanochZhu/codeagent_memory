# 设计

面向 CodeAgent 的记忆模块，用来减少多轮对话里的 token 消耗。同一段代码、同一套解法不该被反复读、反复想。本仓库提供 CLI `cam`：管理项目、代码图和解决方案记忆。Agent 只通过 shell 调用，不做 MCP。

# 记忆

## 代码

用 tree-sitter 把项目解析成图，存在项目内 SQLite（`.cam/cam.db`）。读代码走虚拟文件系统，而不是整文件乱扫。

- `cam index`：解析 Rust / Python / TypeScript / JavaScript / Go
- `cam ls [path]`：目录 / 文件 / 符号
- `cam read <path>`：文件大纲，或符号源码切片（`--full` 才整文件）
- `cam ref <symbol> --dir in|out`：一跳 callers / callees（多跳 hops 第一版不做）

虚拟路径：`src/main.rs` 是文件，`src/main.rs/main` 是该文件里的符号。

## 解决方案

树状记忆，多路召回。探索项目前先 `recall`；没有命中再搜代码，搜到后 `add`。

- `cam recall "<一句话>"`：向量 + BM25，两路分数各自 min-max 到 `[0,1]` 后**求和**（不是 RRF）
- `cam add --summary "..." [--parent ID]`：必须提供全文（stdin / `--file`）和摘要
- 每条记忆带时间。超过 `~/.cam/config.toml` 的 `stale_days`（默认 30）会标 `stale`，考虑是否更新

向量默认 `model2vec` + `potion-multilingual-128M`（首次需下载）。模型不可用时回退到 hash embedder。中文 BM25 先用 jieba 切词再进 FTS5。

## 记忆遗忘

用艾宾浩斯 / MemoryBank 公式算保留率，并加到召回分数上：`R = exp(-t / S)`。`t` 是距 C0（`recalled_at`，没有则用 `created_at`）的天数，初始 `S = 7` 天。

- 召回成功（`needs_update = false`）时刷新 C0，并把 `S *= 1.7`（SM-2 默认难度）
- `R < 0.3` 标 `needs_update`（遗忘带；不用 FSRS 的 0.9，那是复习间隔目标，不是“该重写”）
- 记忆不删除。同一路径上多条并存时标 `latest`，采用更新的那条

## 记忆更新

记忆不会删除，而是根据时间判断是否采用最新的记忆。需要更新时 `add` 一条新节点（可挂 `--parent`），旧节点留在树上。

# Agent 速查

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

全局 `--json`、`--path <project>`。输出默认尽量短。
