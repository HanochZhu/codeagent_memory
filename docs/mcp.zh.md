# MCP 与各 AI 开发工具接入

`cam mcp` 是 **stdio** MCP 服务（一行一条 JSON-RPC）。主 Agent 走 MCP；subagent 继续走 CLI — 见 [docs/agents.zh.md](agents.zh.md)。

```bash
cam mcp
cam --project /path/to/project mcp
```

日志只写 stderr。stdout 只能是 JSON-RPC。

每个工具解析项目根的顺序：参数 `path` → 环境变量 `CAM_PROJECT` → 启动服务时的 `--project` → 向上找 `.cam` / `.git` → 服务进程的当前目录。第一次调用工具时会自动创建 `.cam/` 和数据库。

宿主进程必须能在 PATH 里找到 `cam`。Windows 安装位置一般是 `%USERPROFILE%\.cargo\bin`。找不到就在 `command` 里写 `cam.exe` 的绝对路径。

---

## 工具

| 工具 | 对应 CLI |
| --- | --- |
| `cam_index` | `cam index` |
| `cam_ls` | `cam ls [virt_path]` |
| `cam_read` | `cam read <virt_path> [--full]` |
| `cam_ref` | `cam ref <symbol> --dir in\|out [--file SUBSTR] [--kind KIND] [--scope DIR]` |
| `cam_recall` | `cam recall "<query>" [--limit N] [--fusion rrf\|sum] [--no-expand]` |
| `cam_add` | `cam add --summary "..." [--parent ID] [--body TEXT | --file PATH]` |
| `cam_mem_tree` | `cam mem tree` |
| `cam_mem_show` | `cam mem show <id>` |

`initialize` 会带上 `instructions`，说明 recall → 代码图 → add 的流程。

裸名匹配到多个定义时，`cam_ref` / `cam_read` 返回 `{"status":"ambiguous", "candidates":[…]}`（不是 `isError`）。每个候选带 `id`，可直接回传为 `symbol` / `virt_path`；也可给 `cam_ref` 传 `file`（路径子串）、`kind`、`scope`（子目录，例如 `cam_ls` 里类型为 `project` 的嵌套仓库）收窄。解析成功的响应带 `status: "ok"` 和 `resolved` 块，说明实际使用的节点。

---

## 通用 stdio 配置

多数宿主都能用：

```json
{
  "command": "cam",
  "args": ["mcp"]
}
```

宿主 cwd 不是仓库时，可以写死项目根：

```json
{
  "command": "cam",
  "args": ["--project", "/absolute/path/to/project", "mcp"]
}
```

改完配置后重启宿主。

---

## Cursor

- **项目级：** `.cursor/mcp.json`
- **用户级：** `~/.cursor/mcp.json`（Windows：`%USERPROFILE%\.cursor\mcp.json`）

```json
{
  "mcpServers": {
    "cam": {
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

Cursor Settings → MCP 里确认 `cam` 已启用。主 Agent 能看到 `cam_*`。Task / explore 等 subagent **拿不到**这些工具 — 把 [CLI 说明](agents.zh.md#subagentcli) 交给它们。

---

## Claude Code

- **用户级：** `~/.claude.json`（`mcpServers`）
- **项目级：** `.mcp.json`
- 或：`claude mcp add cam -- cam mcp`

```json
{
  "mcpServers": {
    "cam": {
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

会话里跑 `/mcp`，看不到就重启。工具列表被裁过的 subagent 用 CLI。

---

## Codex

`~/.codex/config.toml`：

```toml
[mcp_servers.cam]
command = "cam"
args = ["mcp"]
```

下次启动 Codex Desktop / CLI 生效。`codex exec` 以及其它子进程用 `cam --json`。

---

## Windsurf

`~/.codeium/windsurf/mcp_config.json`：

```json
{
  "mcpServers": {
    "cam": {
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

Cascade 是主 Agent（MCP）。自定义 / flow subagent 走 CLI。

---

## GitHub Copilot

**VS Code / Copilot Chat** — `.vscode/mcp.json`：

```json
{
  "servers": {
    "cam": {
      "type": "stdio",
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

部分版本仍在用户 `settings.json` 里用 `"mcpServers"`。一种不生效就换另一种。

**Copilot CLI** — `~/.copilot/mcp-config.json`（或 `$COPILOT_HOME/mcp-config.json`）：

```json
{
  "mcpServers": {
    "cam": {
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

---

## Continue

优先 `~/.continue/config.yaml`：

```yaml
mcpServers:
  - name: cam
    command: cam
    args:
      - mcp
```

旧版 `~/.continue/config.json` 的 `mcpServers` 是 `{ name, command, args }` 数组。

---

## Cline

**CLI：** `~/.cline/mcp.json`

```json
{
  "mcpServers": {
    "cam": {
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

**VS Code 扩展：** Cline Settings → MCP Servers → Edit JSON（同一段）。

---

## JetBrains（IntelliJ / RustRover / GoLand）

Settings → Tools → AI Assistant → Model Context Protocol（不同版本名称略有差别）。加一个 stdio 服务：

- Command：`cam`
- Arguments：`mcp`

如果 IDE 启动时 PATH 里还没有 `~/.cargo/bin`，把 `command` 写成 `cam` / `cam.exe` 的绝对路径。

---

## Gemini CLI / Google Antigravity

**Gemini CLI：** `~/.gemini/settings.json` — 标准 `mcpServers`（和 Cursor 一样）。

**Antigravity：**

| 系统 | 文件 |
| --- | --- |
| Windows | `%APPDATA%\Antigravity\User\mcp_config.json` |
| macOS | `~/Library/Application Support/Antigravity/User/mcp_config.json` |
| Linux | `~/.config/Antigravity/User/mcp_config.json` |

信封同样是 `{ "mcpServers": { "cam": { "command": "cam", "args": ["mcp"] } } }`。

---

## OpenCode

`~/.config/opencode/opencode.json` 用顶层 `mcp`（不是 `mcpServers`）：

```json
{
  "mcp": {
    "cam": {
      "type": "local",
      "command": ["cam", "mcp"],
      "enabled": true
    }
  }
}
```

---

## Zed

`~/.config/zed/settings.json` — 键名是 `context_servers`，不是 `mcpServers`：

```json
{
  "context_servers": {
    "cam": {
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

---

## Factory Droid

`~/.factory/mcp.json`（项目覆盖：`.factory/mcp.json`）：

```json
{
  "mcpServers": {
    "cam": {
      "type": "stdio",
      "command": "cam",
      "args": ["mcp"]
    }
  }
}
```

---

## 其它（Qwen Code、Kiro、Warp）

这些宿主也收 stdio MCP。常见路径：

| 宿主 | 配置 |
| --- | --- |
| Qwen Code | `~/.qwen/settings.json` → `mcpServers` |
| Kiro | `~/.kiro/settings/mcp.json` → `mcpServers` |
| Warp | Warp MCP 设置界面，command `cam` / args `mcp` |

宿主要求 `type` 时写成 `"type": "stdio"`。

---

## 自检

```bash
cam --help          # 应列出 mcp
cam mcp             # stderr：`cam mcp 0.1.0 ready (stdio)`
```

再打开宿主的 MCP 面板，确认能看到 `cam_recall`、`cam_ls`、`cam_read`、`cam_ref`、`cam_add`。

仍然没有服务？宿主 PATH 里没有 `cam` — 把 `command` 改成二进制绝对路径并重启。
