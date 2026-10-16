# 事实与代码如何关联

- 日期：2026-10-08
- 问题：cam 是如何将事实和代码关联在一起的？
- 状态：已结论

## 目标
确认「事实」是不是挂在代码符号上，还是两面分开、只在使用时碰到一起。

## 探索路径
1. 读 `docs/index.md` 与 `docs/design/2026-09-16-memory-not-solution.md` → 已确认两面（代码图 / 记忆），事实只是记忆内容的一种，不拆表。文档没写跨面外键。
2. 核对 `src/db.rs` schema → `solutions` 只引用别的记忆（`parent_id` / `supersedes_id`）；`nodes` / `edges` 只引用代码节点。没有记忆→符号的外键。
3. 核对 `src/memory/add.rs`、`src/memory/recall.rs` → `add` 只写入摘要、正文、向量、FTS；`recall` 只查 `solutions`，不 join `nodes`。
4. 核对 `src/code/index.rs` 边类型 → 代码图边是 `contains` / `call` / `implements` / `references`，都在符号之间。
5. 核对 `DESIGN.md` 与 `.opencode/skill/cam/SKILL.md` 工作流 → 关联发生在 Agent 顺序：先 `recall`，未命中再 `ls` / `read` / `ref`，解完再 `add`。

## 关键发现
- **没有结构化关联**：事实不会指向 `nodes.id` 或虚拟路径。正文里出现的文件名、符号名只是文本。
- **同一库、两张互不引用的图**：都在 `.cam/cam.db`。代码图由 tree-sitter 解析写入；记忆由 `cam add` 写入。
- **召回不穿过代码图**：向量 + BM25 + RRF 只在记忆上做。标识符会被拆词，所以正文里写了 `fuse_rrf` 时，用这个名字去 `recall` 可能命中那条记忆，但这是词面重合，不是边。
- **Agent 流程把两面串起来**：命中且 `latest`、非 `needs_update` 就复用记忆；否则走代码图。写回时把结论放进记忆正文，类型（事实 / 习惯 / 设计 / 解法）也写在摘要和正文里，不是字段。

## 结论
cam 不把事实挂到代码符号上。事实是记忆树上的一条普通记录；代码是另一张图。两者靠「先回忆、再读图、再写回」的使用顺序，以及正文里的路径和符号名，在查询时碰到一起。

## 续探（记忆之间有没有图）

- 问题：记忆之间目前是不是没有建图。
- **有树，没有边图（2026-10-08 原状）。** `parent_id` 是可分叉的主题层级，`cam mem tree` 用它组树；`supersedes_id` 有唯一约束，一条记忆最多一个后继，形成线性修订链。当时没有记忆版的 `edges` 表，也没有「相关 / 矛盾 / 提及」这类关系。
- **召回不走结构树。** `revision_tails` 只沿 `supersedes_id` 找到链尾。`breadcrumb` 把父链摘要放进结果的 `path`，供展示，不把父、子、兄弟拉进候选。见 `DESIGN.md`「更新」与 `src/memory/recall/mod.rs`。
- **2026-10-16 更新（原子事实 + 知识图谱边）：** 提取记忆时每条记忆只存一条原子事实，多个相关事实写入时通过 `solution_links`（CLI `--link` / MCP `links`）建立知识图谱关联，但召回阶段暂不做图召回（详见 [../../design/2026-10-16-atomic-fact-links.md](../../design/2026-10-16-atomic-fact-links.md)）。

## 仍开放的问题
- 无

## 相关路径
- `src/db.rs` — `files` / `nodes` / `edges` 与 `solutions` 分表，无跨表外键
- `src/memory/add.rs` — 写入记忆，不接收符号 id
- `src/memory/recall.rs` — 只检索 `solutions`
- `src/code/index.rs` — 代码图边只连接符号
- `DESIGN.md` — 两面与 recall → 代码图 → add 的顺序
- `docs/design/2026-09-16-memory-not-solution.md` — 事实是记忆内容，不是独立存储
