# 原子事实记忆与知识图谱连接

- 日期：2026-10-16
- 来源：agent 对话
- 状态：已确认
- 问题：记忆提取时如何控制单条记忆粒度，以及多个相关事实之间如何建立关联

## 背景

如果一条记忆塞入过多事实或长篇混杂信息，会稀释向量表征与 BM25 词频权重，且后续单点事实变更时难以局部修订。同时，一次任务往往产出多条彼此相关的事实，需要把相关性记录下来形成知识图谱。初版先建立图边；后续 #8 再加入受限的一跳补充召回。

## 决策

- **单条记忆只存一条原子事实**：记忆提取（`cam add` / `cam_add`）时，每条记忆只记录一条事实，避免一条记忆包含过多信息。
- **相关事实建立 link 构成知识图谱**：存储多条相关事实时，通过 `--link <id>`（CLI）或 `links: [<id>, ...]`（MCP `cam_add`）在事实之间建立连接，落地到 SQLite `solution_links` 表。
- **当时暂不做图召回**：初版 `cam recall` 不沿 `solution_links` 扩散，只保留向量 + BM25 + RRF 与 `supersedes_id` 修订链扩展。

## 架构

```text
solutions (单条原子事实)
  ├── parent_id       主题层级树（可分叉）
  ├── supersedes_id   线性版本替代链（唯一后继，recall 扩展链尾）
  └── solution_links  事实间知识图谱边 (source_id, target_id)，mem show 双向展示
```

## 否决方案

- **把多条相关事实合并进同一条记忆正文**：单条记忆信息过载，检索与修订粒度过粗。
- **无界或多跳扩散**：图扩展只走一跳并限制补充结果数量，避免关联记忆淹没直接命中。

## 后续：一跳补充召回（#8）

直接命中的记忆及其最新修订仍按原有相关性排序并占据 `limit` 名额。随后从这些结果出发，沿现有 `solution_links` 双向取一跳邻居，邻居以半分衰减分作为补充结果，并单独受 `limit` 限制。未关联的记忆不会进入补充结果；若关联目标已有 `supersedes_id` 后继，默认补充最新修订。`include_superseded` 保持原有历史查询语义。CLI `cam recall` 与 MCP `cam_recall` 共用同一个实现。

## 相关路径

- `src/db.rs` — `solution_links` 表与索引定义
- `src/memory/add.rs` — `add_solution_with_links` 校验目标记忆并写入 `solution_links`
- `src/memory/tree.rs` — `SolutionView.links` 双向查询关联事实
- `src/memory/recall/mod.rs` — 召回沿 `solution_links` 做一跳补充扩展
- `src/main.rs` / `src/mcp.rs` — CLI `--link` 与 MCP `cam_add.links`
- [../explore/memory/2026-10-08-fact-code-link.md](../explore/memory/2026-10-08-fact-code-link.md) — 早期关于事实与代码、记忆间图结构的探索
