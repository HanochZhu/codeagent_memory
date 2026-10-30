# HaluMem-Medium × cam

[HaluMem](https://arxiv.org/abs/2511.03506) marks atomic memory points on multi-session dialogues, including which new point replaces an older one. `eval/halumem/run.py` downloads `HaluMem-Medium.jsonl` to the path in [datasets.toml](../datasets.toml).

Each session is given to the model with the current one-fact `cam_add` rule. The model writes the memories. A gold point is covered when one stored memory holds at least 55% of its content words. An update is correct when the memory that covers the new text supersedes a memory that covers the replaced text. Interference points that were not stored count as resisted.

`compare.py` scores cam and open-source Mem0 with one DeepSeek model and the HaluMem judge prompts. MemOS needs Neo4j and Qdrant, Memobase needs its server, and Zep Cloud has no API key here. The lexical `run.py` path is a separate check and is not that comparison.

```bash
python3 eval/halumem/test_judge.py
/tmp/halu-venv/bin/python eval/halumem/compare.py --users 2
```

The default lexical run is the first two users, in session order, because scoring every user means one model call per session across 1,387 sessions. `--users 20` is the full Medium set.

```bash
python3 eval/halumem/test_score.py
python3 eval/halumem/run.py
python3 eval/halumem/run.py --users 20
```

`DEEPSEEK_API_KEY` is read from `.deepseek.env` or `.env`. The default model is `deepseek-flash`. Vectors default to `potion-multilingual-128M`; `--hash-embed` selects the hash embedder. The lexical scores do not read the vectors.
