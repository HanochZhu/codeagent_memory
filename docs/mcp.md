# MCP and AI tool support

`cam mcp` is a **stdio** MCP server (newline-delimited JSON-RPC). The main agent talks to it; subagents should keep using the CLI — see [docs/agents.md](agents.md).

```bash
cam mcp
cam --path /path/to/project mcp
```

Logs go to stderr. stdout is JSON-RPC only.

Project resolution for every tool: argument `path` → env `CAM_PROJECT` → `--path` used to start the server → walk up for `.cam` / `.git`.

Need `cam` on the **GUI / IDE PATH**. Windows install location is usually `%USERPROFILE%\.cargo\bin`. If the host cannot find `cam`, put the full path to `cam` / `cam.exe` in `command`.

---

## Tools

| Tool | CLI equivalent |
| --- | --- |
| `cam_init` | `cam init [path]` |
| `cam_index` | `cam index [path]` |
| `cam_ls` | `cam ls [virt_path]` |
| `cam_read` | `cam read <virt_path> [--full]` |
| `cam_ref` | `cam ref <symbol> --dir in\|out` |
| `cam_recall` | `cam recall "<query>" [--limit N] [--fusion rrf\|sum]` |
| `cam_add` | `cam add --summary "..." [--parent ID]` |
| `cam_mem_tree` | `cam mem tree` |
| `cam_mem_show` | `cam mem show <id>` |

`initialize` also returns `instructions` describing the recall → graph → add workflow.

---

## Common stdio block

Most hosts accept this:

```json
{
  "command": "cam",
  "args": ["mcp"]
}
```

Optional default project (useful when the host cwd is not the repo):

```json
{
  "command": "cam",
  "args": ["--path", "/absolute/path/to/project", "mcp"]
}
```

Restart the host after editing MCP config.

---

## Cursor

- **Project:** `.cursor/mcp.json`
- **User:** `~/.cursor/mcp.json` (Windows: `%USERPROFILE%\.cursor\mcp.json`)

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

Cursor Settings → MCP → confirm `cam` is enabled. Main agent tools appear as `cam_*`. Task / explore subagents do **not** get these tools — give them the [CLI sheet](agents.md#subagent-cli).

---

## Claude Code

- **User:** `~/.claude.json` (`mcpServers`)
- **Project:** `.mcp.json`
- Or: `claude mcp add cam -- cam mcp`

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

In a session run `/mcp` and restart if the server does not show up. Subagents started with a limited tool list should use the CLI.

---

## Codex

`~/.codex/config.toml`:

```toml
[mcp_servers.cam]
command = "cam"
args = ["mcp"]
```

Codex Desktop / CLI pick this up on next launch. `codex exec` and other sub-processes should use `cam --json`.

---

## Windsurf

`~/.codeium/windsurf/mcp_config.json`:

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

Cascade is the main agent (MCP). Custom / flow subagents: CLI.

---

## GitHub Copilot

**VS Code / Copilot Chat** — `.vscode/mcp.json`:

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

Some builds still use `"mcpServers"` in user `settings.json`. If one shape is ignored, try the other.

**Copilot CLI** — `~/.copilot/mcp-config.json` (or `$COPILOT_HOME/mcp-config.json`):

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

Prefer `~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: cam
    command: cam
    args:
      - mcp
```

Legacy `~/.continue/config.json` uses an array of `{ name, command, args }` under `mcpServers`.

---

## Cline

**CLI:** `~/.cline/mcp.json`

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

**VS Code extension:** Cline Settings → MCP Servers → Edit JSON (same block).

---

## JetBrains (IntelliJ / RustRover / GoLand)

Settings → Tools → AI Assistant → Model Context Protocol (wording varies by version). Add a stdio server:

- Command: `cam`
- Arguments: `mcp`

If the IDE was started before `~/.cargo/bin` was on PATH, use the absolute path to `cam` / `cam.exe`.

---

## Gemini CLI / Google Antigravity

**Gemini CLI:** `~/.gemini/settings.json` — standard `mcpServers` block (same as Cursor).

**Antigravity:**

| OS | File |
| --- | --- |
| Windows | `%APPDATA%\Antigravity\User\mcp_config.json` |
| macOS | `~/Library/Application Support/Antigravity/User/mcp_config.json` |
| Linux | `~/.config/Antigravity/User/mcp_config.json` |

Same `{ "mcpServers": { "cam": { "command": "cam", "args": ["mcp"] } } }` envelope.

---

## OpenCode

`~/.config/opencode/opencode.json` uses a top-level `mcp` key (not `mcpServers`):

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

`~/.config/zed/settings.json` — key is `context_servers`, not `mcpServers`:

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

`~/.factory/mcp.json` (project override: `.factory/mcp.json`):

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

## Others (Qwen Code, Kiro, Warp)

These also take a stdio MCP entry. Typical files:

| Host | Config |
| --- | --- |
| Qwen Code | `~/.qwen/settings.json` → `mcpServers` |
| Kiro | `~/.kiro/settings/mcp.json` → `mcpServers` |
| Warp | Warp MCP settings UI, command `cam` / args `mcp` |

If the host requires `type`, set `"type": "stdio"`.

---

## Check

```bash
cam --help          # must list `mcp`
cam mcp             # stderr: `cam mcp 0.1.0 ready (stdio)`
```

Then open the host's MCP panel and confirm tools `cam_recall`, `cam_ls`, `cam_read`, `cam_ref`, `cam_add` are listed.

Still no server? The host cannot see `cam` on PATH — switch `command` to the absolute binary path and restart.
