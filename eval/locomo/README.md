# LoCoMo × cam

[LoCoMo](https://github.com/snap-research/locomo) (`locomo10.json`): 10 long conversations. Each dialog turn is one memory. Scoring is evidence recall: the fraction of annotated `dia_id`s (`D1:3`) present in the top-k `cam recall` hits.

This is not the paper's answer F1. Answer F1 needs a reader model. Category ids follow the released eval code: 1 multi-hop, 2 temporal, 3 open-domain, 4 single-hop, 5 adversarial. Questions with an empty evidence list are skipped.

```bash
python3 eval/locomo/run.py --workers 4
python3 eval/locomo/run.py --hash-embed
```

The default embedder is `minishlab/potion-multilingual-128M`. The first run downloads `data/locomo10.json`.
