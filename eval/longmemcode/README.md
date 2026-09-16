# LongMemCode × cam

Adapter + local scorer for [LongMemCode](https://github.com/CataDef/LongMemCode).
Graph facts come from `cam index`; SCIP stable ids are translated from the official scenario file.

```bash
# unit tests (no clone)
python3 eval/longmemcode/test_score.py

# clap (536 scenarios). First run clones clap v4.6.1 and indexes it.
python3 eval/longmemcode/run.py --corpus clap
```

`implementors` / multi-hop subtypes are scored under `deferred_*`. cam now emits `implements` / `references` as well as `calls`; one-hop still cannot do true multi-hop impact.

clap v4.6.1 (536 scenarios, 2026-09-14, this tree):

| slice | n | accuracy |
|---|---:|---:|
| supported (one-hop) | 478 | 0.769 |
| deferred (impl / multi-hop) | 58 | 0.216 |
| raw / weighted | 536 | 0.709 / 0.704 |
| lookup / file_symbols / contained_by | 293 / 55 / 52 | 0.808 / 0.915 / 0.712 |
| callers / callees / implementors | 67 / 18 / 40 | 0.397 / 0.811 / 0.119 |

P95 ≈ 6.5 ms, `$/1k` = 0. Graph: 330 files / 5006 nodes / 43707 edges (calls 24211, references 13313, contains 6089, implements 94).

fastapi 0.115.6 (425 scenarios, 2026-09-15 reindex):

| slice | n | accuracy |
|---|---:|---:|
| supported (one-hop) | 368 | 0.695 |
| deferred | 57 | 0.635 |
| raw / weighted | 425 | 0.687 / 0.714 |
| lookup / file_symbols | 240 / 41 | 0.763 / 0.850 |
| callers / callees / implementors | 58 / 18 / 39 | 0.157 / 0.564 / 0.897 |

Official stdio adapter (after `cam index` on the corpus):

```bash
python3 eval/longmemcode/adapter.py \
  --corpus eval/longmemcode/corpora/clap \
  --scenarios eval/longmemcode/data/clap.json
```

codegraph backend (needs `codegraph` on PATH; first run `codegraph init -y` on the corpus):

```bash
python3 eval/longmemcode/run.py --corpus clap --backend codegraph
```

clap v4.6.1, same scenarios / SCIP table (2026-09-14):

| slice | old cam | cam (this tree) | codegraph |
|---|---:|---:|---:|
| weighted | 0.656 | **0.704** | 0.702 |
| supported (one-hop) | 0.739 | **0.769** | 0.769 |
| lookup | 0.808 | 0.808 | 0.808 |
| callers | 0.090 | **0.397** | 0.379 |
| callees | 0.500 | **0.811** | 0.811 |
| implementors | 0.000 | **0.119** | 0.094 |

Edge kinds (clap sqlite): cam 24211 calls / 13313 references / 6089 contains / 94 implements (43707 total). codegraph 16891 calls / 2750 references / 6942 contains / 45 implements (+ imports/instantiates/extends; 26973 total).
