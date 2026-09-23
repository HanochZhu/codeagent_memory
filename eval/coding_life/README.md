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

# revision chains: one memory per turn, each a --parent revision of the last
python3 eval/coding_life/run.py --hash-embed --lineage
python3 eval/coding_life/run.py --hash-embed --lineage --no-expand
```

Without `--hash-embed`, `CAM_REQUIRE_MODEL2VEC=1` is set so a failed model download does not silently fall back to the hash embedder.

Reports R@5, hit rate, P@5 (ceiling 0.240). `--fusion rrf|sum` selects score fusion.

Comparison on this tree, 2026-09-23 (same `score.ts` formula). Headline tables and charts: [README.md#benchmarks](../../README.md#benchmarks).

| system | n / k | hit rate | R@5 | P@5 |
|---|---|---|---:|---:|
| grep (tokenized substring) | 15 / 5 | 15 / 15 | 0.967 | 0.227 |
| cam sum (min-max) | 15 / 5 | 15 / 15 | 0.933 | 0.213 |
| **cam rrf** (k=60, default) | 15 / 5 | 15 / 15 | **1.000** | **0.240** (ceiling) |
| agentmemory hybrid (published v0.9.26) | 15 / 5 | 15 / 15 | 1.000 | 0.240 |

Grep misses the second gold on `temporal` (q-015). Sum also misses `multi-session-causal` (q-011). RRF recovers both.

## Revision chains

`--lineage` turns each session into a chain of `--parent` revisions and reports two extra metrics: `newest_R@k` (did the top-k carry the newest node of each gold chain) and `stale_latest` (hits flagged `latest` that a newer revision had already superseded).

| k | R@k | P@k | newest R@k | stale `latest` |
|---:|---:|---:|---:|---:|
| 5 | 0.967 | 0.227 | 0.567 | 0 |
| 10 | 1.000 | 0.120 | 0.800 | 0 |

`--no-expand` scores identically at both k on this corpus, so the chain-tail expansion is unmeasured here; it exists for the case where a revision shares no wording with the query, which these transcripts do not produce.
