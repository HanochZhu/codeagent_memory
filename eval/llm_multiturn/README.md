# Multi-turn DeepSeek × cam

Compare a full context dump against cam retrieval in one conversation. Token counts come from DeepSeek `usage` on answering calls (judge calls are recorded separately).

```bash
# needs DEEPSEEK_API_KEY in .env (MODEL / DEEPSEEK_BASE_URL optional)
python3 eval/llm_multiturn/run.py
python3 eval/llm_multiturn/run.py --track solutions
python3 eval/llm_multiturn/run.py --track code
```

`full` puts every session / every `src/*.rs` file in the system prompt. `cam` injects `cam recall` (plus surgical `read` / `ref` on code) for the current turn only; later turns keep Q&A, not retrieved blobs.

Thinking mode is disabled so CoT does not eat `max_tokens`.

deepseek-flash, hash embedder, 2026-09-15:

| track | n | full acc. | cam acc. | full tokens | cam tokens | saving |
|---|---:|---:|---:|---:|---:|---:|
| solutions (coding-agent-life-v1) | 15 | 1.00 | 1.00 | 23673 | 13255 | 44% |
| code (this repo after `cam index`) | 6 | 1.00 | 0.50 | 169838 | 6432 | 96% |

Mean prompt tokens / turn: solutions 1521 → 838; code 28250 → 1004.

Code misses (cam): callers of the fusion helper not in the snippet; `retention` body without `INITIAL_STABILITY_DAYS = 7`; DESIGN.md recall missed the memory update command.

## Locating symbols: scan vs `cam find`

```bash
python3 eval/llm_multiturn/run.py --track code --locate all --no-full
```

`--locate` sets how the code arm finds each identifier from the question before `ref` / `read`. `scan` tries `read <file>/<ident>` on every `src/**/*.rs` file. `find-exact` takes the top `cam find` hit whose name equals the identifier in any case. `find-prefix` also accepts a name that only starts with it. The design memory is now `docs/agents.md`, because cam stores English memories only and DESIGN.md is Chinese, so these runs are not comparable with the table above.

deepseek-flash, hash embedder, 2026-10-12, three runs each:

| locate | correct of 6 (runs) | mean prompt tokens / turn | context build / question |
|---|---|---:|---:|
| scan | 2, 2, 2 | 1001 | 450–1100 ms |
| find-exact | 4, 4, 3 | 1052 | 50–100 ms |
| find-prefix | 3, 3, 3 | 1172 | 50–100 ms |

Most of the accuracy gap is not better code. On c-002 both arms retrieve the same `ref fuse_rrf --dir in` listing `recall`; find-exact only adds an empty `Calls` enum chunk, yet scan answers "not in context" every run. On c-005, find-exact adds the `Cli` struct and the judge accepts a vague answer. Prefix matching pulls in loosely related symbols for English words (`initial` → `initialize_result`, `memory` → `memory_tree_resource`) and costs about 17% more prompt tokens. c-003's gold is outdated: `tokenize_for_fts` no longer segments Chinese.
