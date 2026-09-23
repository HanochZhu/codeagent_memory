# Agent 使用说明

`cam` 有两条入口，按**谁在调用**选：

| 角色 | 怎么用 | 原因 |
| --- | --- | --- |
| **主 Agent**（Cursor / Claude Code / Codex / Windsurf / Copilot 会话） | MCP 工具 `cam_*` | 宿主会注入工具，结果是结构化 JSON |
| **Subagent**（Task / explore / 后台 / 被委派的 CLI agent） | 在项目目录跑 `cam --json …` | 大多数宿主**不会**把 MCP 挂到 subagent |

先接线： [docs/mcp.zh.md](mcp.zh.md)（English: [docs/mcp.md](mcp.md)）。

---

## 共用流程

两条入口顺序一样：

1. **先 recall。** 结果按分数排序，所以要看标志而不是只取第一条：命中且 `latest`、不是 `needs_update` 才直接复用。没有 `latest` 的那条已被更新的修订取代，而那条修订会和它一起返回。
2. **读代码前先确保已建图。** 若 `ls` / `read` / `ref` 提示未建图，对该项目跑一次 `cam_index` / `cam index`。`cam_index` 是全量重建，之后代码变动用 `cam sync`。
3. **未命中 / stale / needs_update**，走代码图：`ls` → `read`（符号路径）→ `ref`。
4. **解完再 add**（需要更新旧节点时挂 `--parent`）。

虚拟路径：`src/main.rs` 是文件；`src/main.rs/main` 是该文件里的符号。

项目根：`--project` / 工具参数 `path` / 环境变量 `CAM_PROJECT` / 向上找 `.cam` 或 `.git`。

---

## 主 Agent（MCP）

宿主加载 `cam` MCP 之后，**直接调工具**。不要再开 shell 跑 `cam`，除非工具不存在或调用失败。

| 工具 | 何时用 |
| --- | --- |
| `cam_recall` | 探索前先查。用用户的语言写一句话。 |
| `cam_ls` | 列目录、文件，或文件里的符号 |
| `cam_read` | 文件大纲，或符号源码。优先 `src/foo.rs/bar`，少用 `full=true` |
| `cam_ref` | 一跳 callers（`dir=in`）或 callees（`dir=out`） |
| `cam_add` | 写入解法。`summary` + 完整 `body`。更新旧节点时带 `parent` |
| `cam_mem_tree` / `cam_mem_show` | 浏览解法树 |
| `cam_index` | 每个仓库做一次（或代码大挪移之后）。`.cam/` 会在首次调用时自动创建。 |

工具返回 JSON 文本。`cam_recall` 在成功命中（`needs_update = false`）时会刷新保留率。图为空时 `cam_ls` / `cam_read` / `cam_ref` 会以 `not_indexed` 失败；先 `cam_index` 一次再重试。

### 贴进主 Agent 的规则 / AGENTS.md

```text
本仓库使用 cam（代码图 + 解法记忆）。

主 Agent：调用 MCP 工具 cam_recall / cam_ls / cam_read / cam_ref / cam_add / cam_mem_tree / cam_mem_show。MCP 可用时不要跑 cam CLI。

流程：先 cam_recall。读代码前若 cam_ls / cam_read / cam_ref 提示未建图，对该项目 cam_index 一次。未命中或 needs_update 时用 cam_ls → cam_read（符号路径）→ cam_ref 走代码图。解完 cam_add（摘要 + 全文；更新旧节点时带 parent）。

Subagent 通常没有 MCP。委派时让它们在项目目录执行 `cam --json`（见 docs/agents.zh.md）。
```

---

## Subagent（CLI）

Task / explore / `claude -p` / Codex exec / Copilot CLI 脚本里的 subagent **通常看不到 MCP**，必须调二进制。

优先 `--json`。工作目录可能不是仓库时，一律带 `--project`。

```bash
cam --json --project <project> recall "如何做 BM25 和向量的多路召回"
cam --json --project <project> ls src/
cam --json --project <project> read src/memory/recall.rs/fuse_scores
cam --json --project <project> ref fuse_scores --dir in
cam --json --project <project> add --summary "..." --file notes.md
# 或：printf '%s' "$BODY" | cam --json --project <project> add --summary "..."
cam --json --project <project> mem tree
cam --json --project <project> mem show <id>
```

不加 `--json` 时是给人看的短文本，不适合程序解析。

### 贴进 subagent 提示词

```text
你没有 cam 的 MCP 工具。请在项目目录用 cam CLI。

cam --json --project <PROJECT> recall "<一句话>"
cam --json --project <PROJECT> ls [virt_path]
cam --json --project <PROJECT> read <virt_path> [--full]
cam --json --project <PROJECT> ref <symbol> --dir in|out
cam --json --project <PROJECT> add --summary "<一行摘要>" --file <path>
cam --json --project <PROJECT> mem tree
cam --json --project <PROJECT> mem show <id>

先 recall。读代码前若提示未建图，先 `cam index` 一次。未命中再 ls → read 符号路径 → ref。解完 add。虚拟路径：文件是 src/main.rs，符号是 src/main.rs/main。
```

### 对照表

| MCP（主 Agent） | CLI（Subagent） |
| --- | --- |
| `cam_index` | `cam index` |
| `cam_ls` | `cam ls [virt_path]` |
| `cam_read` | `cam read <virt_path> [--full]` |
| `cam_ref` | `cam ref <symbol> --dir in\|out` |
| `cam_recall` | `cam recall "<query>" [--limit N] [--fusion rrf\|sum] [--no-expand]` |
| `cam_add` | `cam add --summary "..." [--parent ID] [--body TEXT \| --file PATH]` |
| `cam_mem_tree` | `cam mem tree` |
| `cam_mem_show` | `cam mem show <id>` |

仅 CLI（无 MCP 工具）：`cam sync`、`cam watch`、`cam status`、`cam config get|set stale_days N`。

`--json` 下失败会在 stdout 输出 `{"error":{"code":…,"message":…}}` 并返回非零退出码；`--pretty` 输出缩进 JSON。

---

## 主 Agent 委派时

主 Agent 要把 **CLI 速查、项目路径、以及当前 recall 未命中的信息** 一并交给 subagent。不要假设 subagent 能调 `cam_*` 工具。

---

## Agent Skill

支持 skill 的宿主（opencode、Claude Code）可以直接装 cam 自带的 skill，让 Agent 知道何时用 cam：[`.opencode/skill/cam/SKILL.md`](../.opencode/skill/cam/SKILL.md)。在仓库内工作时 opencode 会自动加载。

装到全局后所有项目都能用：

```bash
# opencode
mkdir -p ~/.config/opencode/skill/cam && cp .opencode/skill/cam/SKILL.md ~/.config/opencode/skill/cam/
# Claude Code
mkdir -p ~/.claude/skills/cam && cp .opencode/skill/cam/SKILL.md ~/.claude/skills/cam/
```

装完重启宿主。
