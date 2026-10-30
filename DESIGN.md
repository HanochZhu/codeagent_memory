# 设计

面向 CodeAgent 的记忆模块，用来减少多轮对话里的 token 消耗。同一段代码、同一条已经记下的结论，不该被反复读、反复想。本仓库提供 CLI `cam`：管理项目、代码图和记忆。

入口分开：**主 Agent 走 MCP**（`cam mcp`，stdio JSON-RPC，工具名 `cam_*`）；**Subagent 直接调 CLI**（`cam --json …`）。大多数宿主不会把 MCP 挂到 Task / explore / 被委派的子进程上。用法见 [docs/agents.zh.md](docs/agents.zh.md)，各 IDE 配置见 [docs/mcp.zh.md](docs/mcp.zh.md)。

cam 有两面：

1. **代码图** — 从仓库解析，不是写进去的。
2. **记忆** — 代码里没有、但需要记下的东西。用户习惯、项目事实、设计方案、解法都放在同一棵树上，不按类型拆开。不叫 solution / 解决方案。

# 代码图

用 tree-sitter 把项目解析成图，存在项目内 SQLite（`.cam/cam.db`）。读代码走虚拟文件系统，而不是整文件乱扫。

- `cam index`：解析 Rust / Python / TypeScript / JavaScript / Go / Java / C / C++ / C#（全量重建）
- `cam sync`：按内容哈希增量更新图
- `cam watch`：监听源文件变化，debounce 后自动 `sync`
- `cam ls [path]`：目录 / 文件 / 符号
- `cam read <path>`：文件大纲，或符号源码切片（`--full` 才整文件）
- `cam ref <symbol> --dir in|out`：一跳 callers / callees（多跳 hops 第一版不做）
- `cam find <query>`：按符号名子串找定义（不区分大小写，只搜名字不搜正文；正文文本仍用 agent 自带 grep）

虚拟路径：`src/main.rs` 是文件，`src/main.rs/main` 是该文件里的符号。

# 记忆

树状与知识图谱结合存储，多路召回。一条记忆可以是习惯、事实、设计或解法，schema 相同；类型写在摘要和正文里。提取记忆时，**一条记忆只存储一条事实，或一个问题的解决路径**（现象、原因、改法、如何验证）。两段内容能分别被问到，或者会分别过时，就分成两条，正文只保留这一条需要的细节。相关的几条用 `--link` / `links` 连成知识图谱（当前暂不做图召回，召回仍只走向量 + BM25 + RRF 及线性修订链扩展）。MCP 的 `cam_add` 可以用 `memories` 一次写入多条，数组里用 `#0`、`#1` 在 `parent`、`supersedes`、`links` 里互指。探索项目前先 `recall`；没有命中再搜代码；需要长期留下的结论再拆成单条 `add`。

- `cam recall "<一句话>" [--no-expand]`：向量 + BM25，以 **RRF**（k=60；分数乘以 k+1，使单路第一名=1、双路第一名=2）融合
- 融合前每路先过门槛：向量分不到本路第一名的 80%、BM25 分不到本路第一名的 30% 的，不算在该路命中。RRF 只看名次，不设门槛时库里每条记忆都会进向量名单，只共享一个 `ts` 这类近零 IDF 词的文档也会进 BM25 名单
- 输出带 `score`（两位小数）；双路命中约为 2，单路约为 1。`relevance`、`vec_score`、`bm25_score` 只在内部排序用，不输出
- 默认每条都给完整正文。`--snippets` 时，第一条及与它并列（融合分 ≥ 第一条的 0.99）的结果给完整正文，其余正文超过 960 字符的只给 240 字符片段并标 `body_truncated`，需要时用 `mem show <id>` 读全文。RRF 分很平（单路第二名是第一名的 0.98），所以只认近乎并列；短正文截断省下的少于一次 `mem show` 的开销，所以保留原文
- 截断默认关闭，因为 LongMemEval 上的 DeepSeek 问答（`eval/recall_payload/llm_qa.py`，86 题）里：截断省 47% token，但准确率从 57% 掉到 45%，丢分的题大多是金标会话被截成片段、Agent 没打开；提示 Agent 必须打开片段后准确率追平，节省只剩 3.7%（每轮工具调用都要重发整个上下文）
- MCP 工具结果是紧凑 JSON（不缩进）。`eval/recall_payload` 量 recall 结果的 token：短正文语料（coding-agent-life）−37%，LongMemEval 长会话 −3.4%；开 `--snippets` 时 k=3 −57%（算上为截断的金标会话补一次 `mem show` 仍 −35%）。排序不变
- BM25 查询去掉中英文虚词（是否、由、或、the、how…）；驼峰标识符入库时整词和拆开的词都存，`bundledToolSchemas` 也能被 `bundled tool schemas` 命中。分词规则变了会用 `PRAGMA user_version` 触发一次 `fts_text` 重建
- `cam add --summary "..." [--parent ID] [--supersedes ID] [--link ID]...`：CLI 一次写一条，正文只写这一条事实或解决路径需要的细节；多个事实相关时用 `--link` 建立知识图谱边（`mem show` 可查看双向关联 `links`，召回暂不遍历 `links`）；`parent` 是可分叉的结构层级，`supersedes` 是线性版本替代。MCP `cam_add` 的 `memories` 可以一次写入多条，并用 `#0`、`#1` 互指
- 每条记忆带时间。超过 `~/.cam/config.toml` 的 `stale_days`（默认 30）会标 `stale`，考虑是否更新

向量默认 `model2vec` + `potion-multilingual-128M`（首次需下载）。`~/.cam/config.toml` 的 `embedding_model`（或环境变量 `CAM_EMBED_MODEL`）可换成 `hash`、另一个 Hugging Face 上的 model2vec 仓库，或本地 model2vec 目录。未显式指定时，默认模型不可用会回退到 hash embedder；显式指定的模型加载失败则直接报错。每条记忆记下产生向量的模型名。换模型后的第一次 `recall` 会用当前模型重算对不上的向量，再参加当次打分，且不改 `updated_at`。查询与记忆摘要要求使用英语，正文支持非拉丁字符；BM25 拆分英语标识符，并同时索引原词和英语词干。

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

主 Agent（MCP）：`cam_recall` → `cam_find` / `cam_ls` / `cam_read` / `cam_ref` → `cam_add`。

Subagent（CLI）：

```text
cam --json index
cam --json sync
cam --json watch
cam --json ls src/
cam --json read src/memory/recall/fuse.rs/fuse_rrf
cam --json ref fuse_rrf --dir in
cam --json recall "how does hybrid recall fuse BM25 and vectors"
cam --json add --summary "..." --parent <structural-id>
cam --json add --summary "..." --supersedes <old-revision-id>
cam --json add --summary "..." --link <related-fact-id>
cam --json mem tree
cam --json mem show <id>
cam --json status
cam mcp
```

全局 `--project <dir>`、`--json`（紧凑）、`--pretty`（美化）。输出默认尽量短；`--json` 下失败输出 `{"error":{"code":…,"message":…}}` 并返回非零退出码。项目根解析：`--project` → `CAM_PROJECT` → 向上查找 `.cam` / `.git`。MCP 服务：`cam mcp`。
