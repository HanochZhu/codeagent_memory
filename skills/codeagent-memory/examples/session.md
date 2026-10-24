# Example pass

User: "Did we already decide how recall combines the two rankings, and where is that code?"

1. Recall, in English:

```text
cam recall "how hybrid recall fuses BM25 and vector rankings"
```

Usable hit: id `mem_7f3a`, `latest`, not `needs_update`. Body says fusion is reciprocal rank fusion in `src/memory/recall/fuse.rs`.

2. Graph is already indexed (`cam status` shows a file count). Read the symbol, not the file:

```text
cam read src/memory/recall/fuse.rs/add_rrf_ranks
cam ref add_rrf_ranks --dir in
```

3. The decision matches the code. Nothing new to store.

Report:

1. Memory: reused `mem_7f3a`.
2. Code: read `src/memory/recall/fuse.rs/add_rrf_ranks`; callers via `--dir in`.
3. Write-back: nothing new to store.

If recall had missed, the same steps still apply, and a settled fact would be stored as one memory:

```text
cam add --summary "Hybrid recall fuses BM25 and vectors with reciprocal rank fusion" --body "add_rrf_ranks in src/memory/recall/fuse.rs merges the two ranked lists."
```
