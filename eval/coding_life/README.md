# coding-agent-life-v1 × cam

Ingest `agentmemory/eval/data/coding-agent-life-v1` sessions with `cam add`, query with `cam recall`.

```bash
# official embedder (potion-multilingual-128M; needs network on first load)
python3 eval/coding_life/run.py

# hash fallback for offline CI (default fusion: RRF)
python3 eval/coding_life/run.py --hash-embed

# min-max sum fusion (previous default)
python3 eval/coding_life/run.py --hash-embed --fusion sum

# tokenized-substring grep baseline (same score.ts formula)
python3 eval/coding_life/run.py --adapter grep
```

Without `--hash-embed`, `CAM_REQUIRE_MODEL2VEC=1` is set so a failed model download does not silently fall back to the hash embedder.

Reports R@5, hit rate, P@5 (ceiling 0.240). `--fusion rrf|sum` selects score fusion.

Comparison on this tree, 2026-09-16 (same `score.ts` formula). Headline tables and charts: [README.md#benchmarks](../../README.md#benchmarks).

| system | n / k | hit rate | R@5 | P@5 |
|---|---|---|---:|---:|
| grep (tokenized substring) | 15 / 5 | 15 / 15 | 0.967 | 0.227 |
| cam sum (min-max) | 15 / 5 | 15 / 15 | 0.933 | 0.213 |
| **cam rrf** (k=60, default) | 15 / 5 | 15 / 15 | **1.000** | **0.240** (ceiling) |
| agentmemory hybrid (published v0.9.26) | 15 / 5 | 15 / 15 | 1.000 | 0.240 |

Grep misses the second gold on `temporal` (q-015). Sum also misses `multi-session-causal` (q-011). RRF recovers both.
