# coding-agent-life-v1 × cam

Ingest `agentmemory/eval/data/coding-agent-life-v1` sessions with `cam add`, query with `cam recall`.

```bash
python3 eval/coding_life/run.py --hash-embed
# optional: real model2vec (needs network on first load)
python3 eval/coding_life/run.py
```

Reports R@5, hit rate, P@5 (ceiling 0.240). `--hash-embed` is the reproducible default for CI; it is weaker than potion-multilingual.
