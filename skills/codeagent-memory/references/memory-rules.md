# Memory rules

- One atomic fact per memory. Connect related facts with `links` / `--link`. Recall does not walk those links yet.
- `summary` is one English line. Non-Latin text in the summary is rejected. The body may use other languages.
- Recall mixes vector search and BM25 with reciprocal rank fusion, then scales by `0.9 + 0.1 × R`, where `R = exp(-t / S)` is the retention factor.
- A score near 2 means both paths matched. A score near 1 means only one path matched.
- `R < 0.3` is `needs_update`. Older than `stale_days` (default 30) is `stale`.
- A hit that is actually returned refreshes retention (`S` is multiplied by 1.7). Expanded or filtered revisions do not.
- Memories are not deleted. `parent` may branch. `supersedes` is a linear chain; its tail is `latest`.
- Default recall returns the newest revision and hides superseded ones. Pass `--include-superseded` for history.
