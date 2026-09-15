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

Code misses (cam): callers of `fuse_scores` not in the snippet; `retention` body without `INITIAL_STABILITY_DAYS = 7`; DESIGN.md recall missed “never delete / `cam add --parent`”.
