# cam commands

Global flags: `--project <dir>` when the working directory is not the repo; `--json` for one JSON object; `--pretty` to indent it. Project root: `--project`, then `CAM_PROJECT`, then walk upward for `.cam` or `.git`.

In `--json` mode, failure prints `{"error":{"code":"...","message":"..."}}` and exits non-zero.

```text
cam index
cam sync
cam watch [--debounce-ms N]
cam ls [virt_path]
cam read <virt_path> [--full]
cam ref <symbol> --dir in|out [--file SUBSTR] [--kind KIND] [--scope DIR]
cam find <query> [--kind KIND] [--limit N]
cam recall "<query>" [--limit N] [--no-expand] [--include-superseded] [--snippets]
cam add --summary "..." [--parent ID] [--supersedes ID] [--link ID]... [--body TEXT | --file PATH]
cam mem tree
cam mem show <id>
cam status
cam config get
cam config set stale_days N
cam config set embedding_model <model>
cam mcp
```

`cam find` matches symbol names only. It does not search text inside function bodies.

`in` lists callers. `out` lists callees. A bare name that matches several definitions returns `status: ambiguous` and `candidates`. Pass a candidate `id` back as the symbol or virtual path.

`cam config` reads and writes `~/.cam/config.toml` (stale window and embedding model name). It does not print environment variables.
