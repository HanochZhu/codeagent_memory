# Optional MCP

The main agent can use a stdio server. Subagents that cannot see MCP keep using `cam --json`.

The host must already find `cam` on PATH. Logs go to stderr. Stdout is JSON-RPC only.

```json
{
  "command": "cam",
  "args": ["mcp"]
}
```

When the host's working directory is not the repo:

```json
{
  "command": "cam",
  "args": ["--project", "/absolute/path/to/project", "mcp"]
}
```

Restart the host after editing its MCP config.

Tool names match the CLI: `cam_index`, `cam_ls`, `cam_find`, `cam_read`, `cam_ref`, `cam_recall`, `cam_add`, `cam_mem_tree`, `cam_mem_show`. `cam_add` can take one memory as `summary` and `body`, or several from the same solution as `memories`. Inside that array, `#0`, `#1`, ... refer to other items for `parent`, `supersedes`, and `links`. The CLI still stores one memory per `cam add`.

Each tool resolves the project as: argument `path`, then `CAM_PROJECT`, then the server `--project`, then a walk up for `.cam` or `.git`, then the server's current directory.
