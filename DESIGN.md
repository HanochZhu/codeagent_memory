# 设计

面向 CodeAgent 的记忆模块，用来减少多轮对话里的 token 消耗。同一段代码、同一条已经记下的结论，不该被反复读、反复想。本仓库提供 CLI `cam`：管理项目、代码图和记忆。

入口分开：**主 Agent 走 MCP**（`cam mcp`，stdio JSON-RPC，工具名 `cam_*`）；**Subagent 直接调 CLI**（`cam --json …`）。大多数宿主不会把 MCP 挂到 Task / explore / 被委派的子进程上。用法见 [docs/agents.zh.md](docs/agents.zh.md)，各 IDE 配置见 [docs/mcp.zh.md](docs/mcp.zh.md)。

cam 有两面：

1. **代码图** — 从仓库解析，不是写进去的。
2. **记忆** — 代码里没有、但需要记下的东西。用户习惯、项目事实、设计方案、解法都放在同一棵树上，不按类型拆开。不叫 solution / 解决方案。

# 代码图

用 tree-sitter 把项目解析成图，存在项目内 SQLite（`.cam/cam.db`）。读代码走虚拟文件系统，而不是整文件乱扫。

- `cam index`：解析 Rust / Python / TypeScript / JavaScript / Go（全量重建）
- `cam sync`：按内容哈希增量更新图
- `cam watch`：监听源文件变化，debounce 后自动 `sync`
- `cam ls [path]`：目录 / 文件 / 符号
- `cam read <path>`：文件大纲，或符号源码切片（`--full` 才整文件）
- `cam ref <symbol> --dir in|out`：一跳 callers / callees（多跳 hops 第一版不做）

虚拟路径：`src/main.rs` 是文件，`src/main.rs/main` 是该文件里的符号。

# 记忆

树状存储，多路召回。一条记忆可以是习惯、事实、设计或解法，schema 相同；类型写在摘要和正文里。探索项目前先 `recall`；没有命中再搜代码；需要长期留下的结论再 `add`。

- `cam recall "<一句话>"`：向量 + BM25，默认 **RRF**（k=60；分数乘以 k+1，使单路第一名=1、双路第一名=2）。`--fusion sum` 则两路 min-max 到 `[0,1]` 后求和
- `cam add --summary "..." [--parent ID]`：必须提供全文（stdin / `--file`）和摘要
- 每条记忆带时间。超过 `~/.cam/config.toml` 的 `stale_days`（默认 30）会标 `stale`，考虑是否更新

向量默认 `model2vec` + `potion-multilingual-128M`（首次需下载）。模型不可用时回退到 hash embedder。中文 BM25 先用 jieba 切词再进 FTS5。

## 遗忘

用艾宾浩斯 / MemoryBank 公式算保留率，并加到召回分数上：`R = exp(-t / S)`。`t` 是距 C0（`recalled_at`，没有则用 `created_at`）的天数，初始 `S = 7` 天。

- 召回成功（`needs_update = false`）时刷新 C0，并把 `S *= 1.7`（SM-2 默认难度）
- `R < 0.3` 标 `needs_update`（遗忘带；不用 FSRS 的 0.9，那是复习间隔目标，不是“该重写”）
- 记忆不删除。同一路径上多条并存时标 `latest`，采用更新的那条

## 更新

记忆不会删除，而是根据时间判断是否采用最新的记忆。需要更新时 `add` 一条新节点（可挂 `--parent`），旧节点留在树上。

# Agent 速查

主 Agent（MCP）：`cam_recall` → `cam_ls` / `cam_read` / `cam_ref` → `cam_add`。

Subagent（CLI）：

```text
cam --json index
cam --json sync
cam --json watch
cam --json ls src/
cam --json read src/memory/recall.rs/fuse_scores
cam --json ref fuse_scores --dir in
cam --json recall "如何做 BM25 和向量的多路召回"
cam --json add --summary "..." --parent <id>
cam --json mem tree
cam --json mem show <id>
cam --json status
cam mcp
```

全局 `--project <dir>`、`--json`（紧凑）、`--pretty`（美化）。输出默认尽量短；`--json` 下失败输出 `{"error":{"code":…,"message":…}}` 并返回非零退出码。项目根解析：`--project` → `CAM_PROJECT` → 向上查找 `.cam` / `.git`。MCP 服务：`cam mcp`。
