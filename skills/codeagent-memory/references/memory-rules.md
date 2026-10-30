# Memory rules

- One memory is one fact, or the solution path for one problem (symptom, cause, change, and how to verify). If two pieces can be asked about separately, or can become outdated separately, store them as two memories. The body holds only the detail that one memory needs. Connect related memories with `links` / `--link`. On MCP, one `cam_add` can pass a `memories` array and use `#0`, `#1`, ... for other items in that call. Recall does not walk those links yet.
- `summary` is one English line. Non-Latin text in the summary is rejected. The body may use other languages.
- Recall mixes vector search and BM25 with reciprocal rank fusion, then scales by `0.9 + 0.1 × R`, where `R = exp(-t / S)` is the retention factor.
- A score near 2 means both paths matched. A score near 1 means only one path matched.
- `R < 0.3` is `needs_update`. Older than `stale_days` (default 30) is `stale`.
- A hit that is actually returned refreshes retention (`S` is multiplied by 1.7). Expanded or filtered revisions do not.
- Memories are not deleted. `parent` may branch. `supersedes` is a linear chain; its tail is `latest`.
- Default recall returns the newest revision and hides superseded ones. Pass `--include-superseded` for history.
