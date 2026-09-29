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

- `cam recall "<一句话>" [--no-expand]`：向量 + BM25，默认 **RRF**（k=60；分数乘以 k+1，使单路第一名=1、双路第一名=2）。`--fusion sum` 则两路 min-max 到 `[0,1]` 后求和
- 融合前每路先过门槛：向量分不到本路第一名的 80%、BM25 分不到本路第一名的 30% 的，不算在该路命中。RRF 只看名次，不设门槛时库里每条记忆都会进向量名单，只共享一个 `ts` 这类近零 IDF 词的文档也会进 BM25 名单
- 输出带 `relevance`（融合分）、`vec_score`、`bm25_score`；双路命中约为 2，单路约为 1
- BM25 查询去掉中英文虚词（是否、由、或、the、how…）；驼峰标识符入库时整词和拆开的词都存，`bundledToolSchemas` 也能被 `bundled tool schemas` 命中。分词规则变了会用 `PRAGMA user_version` 触发一次 `fts_text` 重建
- `cam add --summary "..." [--parent ID] [--supersedes ID]`：必须提供全文（stdin / `--file`）和摘要；`parent` 是可分叉的结构层级，`supersedes` 是线性版本替代
- 每条记忆带时间。超过 `~/.cam/config.toml` 的 `stale_days`（默认 30）会标 `stale`，考虑是否更新

向量默认 `model2vec` + `potion-multilingual-128M`（首次需下载）。模型不可用时回退到 hash embedder。查询、记忆摘要和正文要求使用英语；BM25 拆分英语标识符，并同时索引原词和英语词干。

## 遗忘

用艾宾浩斯 / MemoryBank 公式算保留率：`R = exp(-t / S)`。`t` 是距 C0（`recalled_at`，没有则用 `created_at`）的天数，初始 `S = 7` 天。

排序分是 `relevance × (0.9 + 0.1 × R)`：保留率最多改变 10%。它说明记忆新不新，不说明它答不答得上这个问题；早先直接把 `R` 加到融合分上，R 与 RRF 分同量级，常被召回的记忆会压过更相关的一条，而每次被返回又会再加强一次，形成正反馈。

- 召回成功（`needs_update = false`）时刷新 C0，并把 `S *= 1.7`（SM-2 默认难度）。仅靠链尾扩展被拉进来的节点不算召回，不刷新
- `R < 0.3` 标 `needs_update`（遗忘带；不用 FSRS 的 0.9，那是复习间隔目标，不是“该重写”）
- 记忆不删除。每条 `supersedes_id` 修订链的最后节点标 `latest`

## 更新

记忆不会删除。`parent_id` 只构建允许分叉的主题/推导层级；需要完整替代旧结论时，`add --supersedes <old-id>` 写一条新节点，旧节点保留用于历史查询。若没有显式传 `--parent`，新版自动继承旧版的结构父节点。

`supersedes_id` 带唯一约束，因此版本替代保持线性；结构树仍可任意分叉。召回只沿 `supersedes_id` 向前找到链尾，不会跨到结构兄弟节点。链尾即使两路都匹配不上也会被拉进候选集，旧修订默认在 top-k 前折叠；`--no-expand` 可关闭扩展，`--include-superseded` 可保留历史。

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
cam --json recall "how does hybrid recall fuse BM25 and vectors"
cam --json add --summary "..." --parent <structural-id>
cam --json add --summary "..." --supersedes <old-revision-id>
cam --json mem tree
cam --json mem show <id>
cam --json status
cam mcp
```

全局 `--project <dir>`、`--json`（紧凑）、`--pretty`（美化）。输出默认尽量短；`--json` 下失败输出 `{"error":{"code":…,"message":…}}` 并返回非零退出码。项目根解析：`--project` → `CAM_PROJECT` → 向上查找 `.cam` / `.git`。MCP 服务：`cam mcp`。
