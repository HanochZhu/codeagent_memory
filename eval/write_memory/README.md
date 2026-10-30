# Model-written memories

The notes are not in this repository. They are the case set at [cam-write-memory-eval](https://github.com/HanochZhu/cam-write-memory-eval). The first run downloads `cases.json` to the path in [datasets.toml](../datasets.toml) (`eval/write_memory/data/cases.json`, or `CAM_EVAL_WRITE_MEMORY`).

A model receives one note and calls `cam_add` itself. `legacy` stores one full write-up. `split` stores one fact, or one problem's solution path, and can link several memories from the same call with `#0`. Scoring is lexical: a fact is exclusive when one memory holds all of its required phrases and no other fact. The design of the four notes is in the case-set README.

```bash
python3 eval/write_memory/test_score.py
python3 eval/write_memory/run.py
```

`DEEPSEEK_API_KEY` is read from `.deepseek.env` or `.env`. The default model is `deepseek-flash`. `MODEL` overrides it. Writes go through `cam mcp` with the hash embedder, into a throwaway directory. The `cam` binary must accept a `memories` array.
