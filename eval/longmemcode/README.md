# LongMemCode × cam

Adapter + local scorer for [LongMemCode](https://github.com/CataDef/LongMemCode).
Graph facts come from `cam index`; SCIP stable ids are translated from the official scenario file.

```bash
# unit tests (no clone)
python3 eval/longmemcode/test_score.py

# clap (536 scenarios). First run clones clap v4.6.1 and indexes it.
python3 eval/longmemcode/run.py --corpus clap
```

`implementors` / multi-hop subtypes are scored but reported under `deferred_*` — cam v1 is one-hop only.

clap v4.6.1 (536 scenarios, 2026-09-14, this tree):

| slice | n | accuracy |
|---|---:|---:|
| supported (one-hop) | 478 | 0.739 |
| deferred (impl / multi-hop) | 58 | 0.017 |
| raw / weighted | 536 | 0.661 / 0.656 |
| lookup / file_symbols / contained_by | 293 / 55 / 52 | 0.808 / 0.915 / 0.793 |
| callers / implementors | 67 / 40 | 0.090 / 0.000 |

P95 ≈ 6.4 ms, `$/1k` = 0. Callers stay low because cam stores `calls`, not SCIP references.

fastapi 0.115.6 (425 scenarios):

| slice | n | accuracy |
|---|---:|---:|
| supported (one-hop) | 368 | 0.704 |
| deferred | 57 | 0.021 |
| raw / weighted | 425 | 0.612 / 0.642 |
| lookup / file_symbols | 240 / 41 | 0.763 / 0.850 |

Official stdio adapter (after `cam index` on the corpus):

```bash
python3 eval/longmemcode/adapter.py \
  --corpus eval/longmemcode/corpora/clap \
  --scenarios eval/longmemcode/data/clap.json
```
