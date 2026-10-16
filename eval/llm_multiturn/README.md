# Multi-turn DeepSeek × cam

Compare a full context dump against cam retrieval in one conversation. Token counts come from DeepSeek `usage` on answering calls (judge calls are recorded separately).

```bash
# needs DEEPSEEK_API_KEY in .env (MODEL / DEEPSEEK_BASE_URL optional)
# default recall vector is potion-multilingual-128M and errors if the model fails to load
# add --hash-embed for the hash tables below
python3 eval/llm_multiturn/run.py
python3 eval/llm_multiturn/run.py --track solutions
python3 eval/llm_multiturn/run.py --track code
```

`full` puts every session / every `src/*.rs` file in the system prompt. `cam` injects `cam recall` (plus surgical `read` / `ref` on code) for the current turn only; later turns keep Q&A, not retrieved blobs.

Thinking mode is disabled so CoT does not eat `max_tokens`.

deepseek-flash, hash embedder, 2026-10-13:

| track | n | full acc. | cam acc. | full tokens | cam tokens | saving |
|---|---:|---:|---:|---:|---:|---:|
| solutions (coding-agent-life-v1) | 15 | 1.00 | 0.87 | 22796 | 9904 | 57% |
| code (this repo after `cam index`) | 6 | 0.83 | 0.33 | 379066 | 6342 | 98% |

Mean prompt tokens / turn: solutions 1471 → 616; code 63076 → 1000.

Code misses (cam): callers of `fuse_rrf`; what `tokenize_for_fts` does for Chinese; `retention` body without `INITIAL_STABILITY_DAYS = 7`; the `cam add` update rule. The full dump also missed the Chinese-tokenization question, whose gold still expects segmentation the function no longer does. Memory cam missed q-002 and q-013.

## Locating symbols: scan vs `cam find`

```bash
python3 eval/llm_multiturn/run.py --track code --locate all --no-full
```

`--locate` sets how the code arm finds each identifier from the question before `ref` / `read`. `scan` tries `read <file>/<ident>` on every `src/**/*.rs` file. `find-exact` takes the top `cam find` hit whose name equals the identifier in any case. `find-prefix` also accepts a name that only starts with it. The design memory is now `docs/agents.md`, because cam stores English memories only and DESIGN.md is Chinese, so these runs are not comparable with the table above.

deepseek-flash, hash embedder, three runs on 2026-10-13. Context-build ranges are from 2026-10-12; this rerun did not log them.

| locate | correct of 6 (runs) | mean prompt tokens / turn | context build / question |
|---|---|---:|---:|
| scan | 3, 2, 2 | 997 | 450–1100 ms |
| find-exact | 3, 4, 4 | 1043 | 50–100 ms |
| find-prefix | 2, 3, 3 | 1172 | 50–100 ms |

Most of the accuracy gap is not better code. On c-002 both arms retrieve the same `ref fuse_rrf --dir in` listing `recall`; find-exact only adds an empty `Calls` enum chunk, yet scan misses every run. On c-005, find-exact is accepted on two of the three runs. Prefix matching pulls in loosely related symbols for English words (`initial` → `initialize_result`, `memory` → `memory_tree_resource`) and costs about 18% more prompt tokens. c-003's gold is outdated: `tokenize_for_fts` no longer segments Chinese.
