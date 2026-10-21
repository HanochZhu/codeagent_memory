# Model-written memories

The other evals call `cam add` with text the harness already wrote. This one gives the note to a model and lets the model call `cam_add`.

Two arms:

| arm | instruction | tool |
| --- | --- | --- |
| `legacy` | one memory, body is the full write-up | `summary` + `body` only, and the model may call it more than once |
| `split` | one fact, or one problem's solution path; split pieces that can be asked about separately or can go stale separately | same fields, plus a `memories` array with `#0` references |

A fact is **exclusive** when one stored memory contains all of its required phrases and no other fact's phrases. A required link counts when those memories share an edge, or both facts landed in the same memory. `lsp-toast` is one solution path, so splitting it lowers the exclusive rate.

```bash
python3 eval/write_memory/test_score.py
python3 eval/write_memory/run.py
```

`DEEPSEEK_API_KEY` is read from `.deepseek.env` or `.env`. The default model is `deepseek-flash`. `MODEL` overrides it. Writes go through `cam mcp` with the hash embedder, into a throwaway directory.
