# codeagent-memory

Local code graph and solution memory for coding agents. The agent recalls past project facts, then reads symbols, callers, and callees through the `cam` CLI instead of opening whole files.

## Before first use

`cam` must already be on PATH (`cam --help`). This package does not include the binary. See `references/install.md`. The database is `.cam/cam.db` in the project you point `cam` at. No API key.

## Ask the agent

- "Was this bug solved before?"
- "Who calls this function, and what does it call?"
- "Read this symbol, not the whole file."
- "Save this decision in project memory."

Recall queries and memory summaries are English. Memory bodies may use other languages.

## In this folder

| Path | Role |
| --- | --- |
| `SKILL.md` | Steps the agent follows |
| `references/commands.md` | CLI flags |
| `references/memory-rules.md` | What one memory may contain |
| `references/mcp.md` | Optional stdio MCP config |
| `references/install.md` | One-time CLI install, run by the user |
| `examples/session.md` | One recall, read, and write-back pass |

License: MIT (`LICENSE.txt`).
