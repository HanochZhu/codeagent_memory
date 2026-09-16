# coding-agent-life-v1 × cam

Ingest `agentmemory/eval/data/coding-agent-life-v1` sessions with `cam add`, query with `cam recall`.

```bash
# official embedder (potion-multilingual-128M; needs network on first load)
python3 eval/coding_life/run.py

# hash fallback for offline CI (default fusion: RRF)
python3 eval/coding_life/run.py --hash-embed

# min-max sum fusion (previous default)
python3 eval/coding_life/run.py --hash-embed --fusion sum
```

Without `--hash-embed`, `CAM_REQUIRE_MODEL2VEC=1` is set so a failed model download does not silently fall back to the hash embedder.

Reports R@5, hit rate, P@5 (ceiling 0.240). `--fusion rrf|sum` selects score fusion.

hash embedder, 2026-09-16 (this tree):

| fusion | n / k | hit rate | R@5 | P@5 | p50 |
|---|---|---|---:|---:|---:|
| sum (min-max) | 15 / 5 | 15 / 15 | 0.933 | 0.213 | 1.4 s |
| **rrf** (k=60, default) | 15 / 5 | 15 / 15 | **1.000** | **0.240** (ceiling) | 1.4 s |

Sum partial misses: second gold on `temporal` (q-015) and `multi-session-causal` (q-011). RRF recovers both.
