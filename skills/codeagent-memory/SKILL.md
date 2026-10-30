---
name: codeagent-memory
description: "Query a local code graph and project memory with the cam CLI. Use when the user asks whether a problem was solved before, wants callers or callees, wants to read a symbol without opening the whole file, mentions .cam/ or cam, or asks to save a project fact for later."
---

# codeagent-memory

`cam` keeps a code graph and solution memory in `.cam/cam.db` inside the project. It is local SQLite. This skill does not download software, read credentials, or send project data anywhere.

## Prerequisite

`cam` must already be on PATH. Check with `cam --help`.

If that fails, stop and ask the user to install it. Do not install packages, change PATH, or fetch an installer. Install notes are in [references/install.md](references/install.md).

## Steps

1. Recall first. Turn the question into one English sentence, then `cam recall "<sentence>"` (or MCP `cam_recall`).
   - Reuse a hit only when it is `latest` and not `needs_update`.
   - Hits carry full bodies. Only with `snippets` / `--snippets` do long lower-ranked bodies come back clipped with `body_truncated`; read one with `cam mem show <id>` before using the details.
   - On `stale`, `needs_update`, or no hit, continue.
2. Index once if `ls`, `find`, `read`, or `ref` says the graph is not indexed: `cam index`. Later file edits use `cam sync`, not another full index.
3. Walk the graph: `cam find <part of a name>` or `cam ls <dir>`, then `cam read <file>/<symbol>`, then `cam ref <symbol> --dir in|out`.
   - `src/main.rs` is a file. `src/main.rs/main` is a symbol in that file.
   - On `status: ambiguous`, retry with a candidate `id`, or narrow with `--file`, `--kind`, or `--scope`.
4. After a new fact is settled, store one memory: one fact, or the solution path for one problem (symptom, cause, change, and how to verify). If two pieces can be asked about separately, or can become outdated separately, store them as two memories. The CLI stores one memory per call: `cam add --summary "<one English line>" --body "<write-up>"`, then `--link <id>` for a related memory. MCP `cam_add` can store several from the same solution in one `memories` array; `#0`, `#1`, ... refer to other items for `links`, `parent`, and `supersedes`. `--parent` is hierarchy. `--supersedes` only when this memory fully replaces an older one.

Prefer MCP tools when they exist: `cam_recall`, `cam_index`, `cam_ls`, `cam_find`, `cam_read`, `cam_ref`, `cam_add`, `cam_mem_tree`, `cam_mem_show`. A subagent with only a shell uses `cam --json`.

Details: [references/commands.md](references/commands.md), [references/memory-rules.md](references/memory-rules.md), [references/mcp.md](references/mcp.md). A full pass is in [examples/session.md](examples/session.md).

## Output

Report in this order:

1. Memory: hit id and whether it was reused, or `no usable memory`.
2. Code: virtual paths read, and whether each `ref` was callers (`in`) or callees (`out`).
3. Write-back: new memory id, or `nothing new to store`.

Quote the symbol body `cam` returned. Do not paste whole files.
