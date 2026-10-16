# 文档索引

## 使用 / 接入

| topic | 文档 | 一句话 |
|---|---|---|
| agent | [agents.zh.md](agents.zh.md) | 主 Agent 走 MCP、Subagent 走 CLI 的分工与工作流 |
| mcp | [mcp.zh.md](mcp.zh.md) | 各 IDE / 宿主的 MCP 配置 |
| skill | [../.opencode/skill/cam/SKILL.md](../.opencode/skill/cam/SKILL.md) | opencode / Claude Code 的 cam skill |

## 探索

| topic | 文档 | 一句话 |
|---|---|---|
| eval | [2026-09-15-multi-turn-token-benchmark.md](explore/eval/2026-09-15-multi-turn-token-benchmark.md) | 记忆面 / 代码图可用的 benchmark，以及 DeepSeek 多轮问答 + token 对比 |
| eval | [2026-09-16-memory-comparison.md](explore/eval/2026-09-16-memory-comparison.md) | cam 与 grep / agentmemory / codegraph 的同语料对照，以及 README 折线图 |
| memory | [2026-10-08-fact-code-link.md](explore/memory/2026-10-08-fact-code-link.md) | 事实与代码无外键；记忆间早期为树+修订链，后增 `solution_links` 知识图谱边 |

## 设计 / 架构

| 文档 | 一句话 |
|---|---|
| [2026-09-16-memory-not-solution.md](design/2026-09-16-memory-not-solution.md) | 非代码面叫记忆 / memory，不叫 solution；习惯、事实、设计、解法同一棵树 |
| [2026-10-16-atomic-fact-links.md](design/2026-10-16-atomic-fact-links.md) | 一条记忆只存一条原子事实，相关事实通过 `links` 建成知识图谱，暂不做图召回 |

## 任务 / 计划

（无）
