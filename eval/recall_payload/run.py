#!/usr/bin/env python3
"""Tokens a cam_recall result costs the main agent, before vs after a change.

Arms, all through `cam mcp` (the text the agent actually reads):

  baseline — `--baseline` binary, default recall
  slim     — candidate binary, default recall (field slimming only)
  tiered   — candidate binary with `snippets: true` (slimming + snippets below the top)

`tiered+show` adds what an oracle agent then pays: one `cam_mem_show` for every
gold memory that came back as a snippet. Ranking is the same in every arm, so
R@k is reported only to show nothing moved.

Corpora:
  coding_life — coding-agent-life-v1, short session bodies (87-361 chars)
  longmemeval — LongMemEval-S, whole chat sessions as bodies (needs the
                dataset at eval/longmemeval/data, fetched by eval/longmemeval/run.py)

Usage:
  python3 eval/recall_payload/run.py --baseline /tmp/old/cam --corpus coding_life
  python3 eval/recall_payload/run.py --baseline /tmp/old/cam --corpus longmemeval --sample 50
"""

from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from eval_paths import cam_binary, dataset  # noqa: E402

HERE = Path(__file__).resolve().parent

try:
    import tiktoken

    _ENC = tiktoken.get_encoding("cl100k_base")

    def count_tokens(text: str) -> int:
        return len(_ENC.encode(text))

    TOKENIZER = "cl100k_base"
except ImportError:

    def count_tokens(text: str) -> int:
        return (len(text.encode("utf-8")) + 3) // 4

    TOKENIZER = "bytes/4"


class Mcp:
    def __init__(self, executable: Path, project: Path) -> None:
        self.next_id = 1
        self.process = subprocess.Popen(
            [str(executable), "mcp", "--project", str(project)],
            cwd=project,
            env={**os.environ, "CAM_HASH_EMBED": "1"},
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.request(
            "initialize",
            {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "cam-recall-payload", "version": "1"},
            },
        )
        self._write({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

    def __enter__(self) -> "Mcp":
        return self

    def __exit__(self, *_args: object) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()

    def request(self, method: str, params: dict) -> dict:
        rid = self.next_id
        self.next_id += 1
        self._write({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        line = self.process.stdout.readline() if self.process.stdout else ""
        if not line:
            raise RuntimeError(f"cam mcp exited during {method}")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"{method}: {response['error']}")
        return response["result"]

    def call_text(self, name: str, arguments: dict) -> str:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        text = result["content"][0]["text"]
        if result.get("isError"):
            raise RuntimeError(text)
        return text

    def _write(self, payload: dict) -> None:
        assert self.process.stdin
        self.process.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
        self.process.stdin.flush()


def load_coding_life(path: Path) -> list[dict]:
    sessions = json.loads((path / "sessions.json").read_text())
    queries = json.loads((path / "queries.json").read_text())
    memories = [
        {
            "key": s["id"],
            "summary": f"{s['id']}: {s['content'].splitlines()[0][:80]}",
            "body": f"{s['id']}\n{s.get('timestamp') or ''}\n{s['content']}",
        }
        for s in sessions
    ]
    return [
        {
            "memories": memories,
            "queries": [
                {"question": q["question"], "gold": set(q["goldSessionIds"])} for q in queries
            ],
        }
    ]


def load_longmemeval(path: Path, sample: int, seed: int) -> list[dict]:
    if not path.exists():
        raise SystemExit(f"{path} missing; run eval/longmemeval/run.py once to fetch it")
    data = json.loads(path.read_bytes())
    eligible = [
        x for x in data if x["answer_session_ids"] and not x["question_id"].endswith("_abs")
    ]
    groups = []
    for inst in random.Random(seed).sample(eligible, min(sample, len(eligible))):
        memories = []
        for sid, date, turns in zip(
            inst["haystack_session_ids"],
            inst["haystack_dates"],
            inst["haystack_sessions"],
            strict=True,
        ):
            lines = [f"SESSION_ID: {sid}", f"DATE: {date}"]
            lines += [f"{t.get('role', 'unknown').upper()}: {t.get('content', '')}" for t in turns]
            memories.append(
                {"key": sid, "summary": f"Session {sid} at {date}", "body": "\n".join(lines)}
            )
        groups.append(
            {
                "instance": inst,
                "memories": memories,
                "queries": [
                    {"question": inst["question"], "gold": set(inst["answer_session_ids"])}
                ],
            }
        )
    return groups


def key_of(hit: dict, keys: list[str]) -> str | None:
    summary = hit.get("summary") or ""
    for key in keys:
        if key in summary:
            return key
    return None


def measure(mcp: Mcp, group: dict, query: dict, limit: int, snippets: bool) -> dict:
    args = {"query": query["question"], "limit": limit, "hash_embed": True}
    if snippets:
        args["snippets"] = True
    text = mcp.call_text("cam_recall", args)
    hits = json.loads(text)
    keys = [m["key"] for m in group["memories"]]
    ranked = [key_of(h, keys) for h in hits]
    truncated_gold = [
        h for h, k in zip(hits, ranked) if k in query["gold"] and h.get("body_truncated")
    ]
    show_tokens = 0
    for h in truncated_gold:
        show_args = {"id": h["id"]}
        show_tokens += count_tokens(json.dumps({"name": "cam_mem_show", "arguments": show_args}))
        show_tokens += count_tokens(mcp.call_text("cam_mem_show", show_args))
    return {
        "tokens": count_tokens(text),
        "compact_tokens": count_tokens(json.dumps(hits, ensure_ascii=False, separators=(",", ":"))),
        "show_tokens": show_tokens,
        "hits": len(hits),
        "truncated": sum(1 for h in hits if h.get("body_truncated")),
        "gold_truncated": len(truncated_gold),
        "recall": len(query["gold"] & {k for k in ranked if k}) / len(query["gold"]),
    }


def ingest(cam: Path, root: Path, memories: list[dict]) -> bool:
    """False when cam rejects a memory (non-Latin text); the group is skipped."""
    with Mcp(cam, root) as mcp:
        for m in memories:
            try:
                mcp.call_text(
                    "cam_add", {"summary": m["summary"], "body": m["body"], "hash_embed": True}
                )
            except RuntimeError as exc:
                if "must be written in English" in str(exc):
                    return False
                raise
    return True


def run(baseline: Path, candidate: Path, groups: list[dict], limit: int) -> dict:
    arms = {
        "baseline": (baseline, False),
        "slim": (candidate, False),
        "tiered": (candidate, True),
    }
    rows: dict[str, list[dict]] = {name: [] for name in arms}
    skipped = 0
    for gi, group in enumerate(groups, 1):
        with tempfile.TemporaryDirectory(prefix="cam-payload-") as tmp:
            seed = Path(tmp) / "seed"
            seed.mkdir()
            if not ingest(candidate, seed, group["memories"]):
                skipped += 1
                continue
            for name, (cam, snippets) in arms.items():
                # Recall refreshes retention, so each arm gets an untouched copy.
                root = Path(tmp) / name
                shutil.copytree(seed, root)
                with Mcp(cam, root) as mcp:
                    for q in group["queries"]:
                        rows[name].append(measure(mcp, group, q, limit, snippets))
        if len(groups) > 1 and gi % 10 == 0:
            print(f"  {gi}/{len(groups)} groups", flush=True)

    def total(name: str, field: str) -> int:
        return sum(r[field] for r in rows[name])

    base = total("baseline", "tokens")
    summary = {
        "queries": len(rows["baseline"]),
        "skipped_non_english": skipped,
        "limit": limit,
        "tokenizer": TOKENIZER,
    }
    for name in arms:
        t = total(name, "tokens")
        summary[name] = {
            "tokens": t,
            "vs_baseline": round(t / base - 1, 4) if base else 0.0,
            "compact_tokens": total(name, "compact_tokens"),
            "mean_tokens": round(statistics.mean(r["tokens"] for r in rows[name]), 1),
            "R@k": round(statistics.mean(r["recall"] for r in rows[name]), 4),
            "truncated_hits": total(name, "truncated"),
            "hits": total(name, "hits"),
        }
    with_show = total("tiered", "tokens") + total("tiered", "show_tokens")
    summary["tiered+show"] = {
        "tokens": with_show,
        "vs_baseline": round(with_show / base - 1, 4) if base else 0.0,
        "show_calls": total("tiered", "gold_truncated"),
        "queries_needing_show": sum(1 for r in rows["tiered"] if r["gold_truncated"]),
    }
    return {"summary": summary, "rows": rows}


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--baseline", type=Path, required=True, help="cam binary before the change")
    p.add_argument("--candidate", type=Path, default=None, help="defaults to CAM_BIN / target")
    p.add_argument("--corpus", choices=("coding_life", "longmemeval"), default="coding_life")
    p.add_argument("--limit", type=int, default=3)
    p.add_argument("--sample", type=int, default=50, help="longmemeval instances")
    p.add_argument("--seed", type=int, default=42)
    args = p.parse_args()
    candidate = (args.candidate or cam_binary()).resolve()
    baseline = args.baseline.resolve()

    if args.corpus == "coding_life":
        groups = load_coding_life(dataset("coding_life"))
    else:
        groups = load_longmemeval(dataset("longmemeval"), args.sample, args.seed)

    out = run(baseline, candidate, groups, args.limit)
    out["summary"]["corpus"] = args.corpus
    results = HERE / "results"
    results.mkdir(exist_ok=True)
    dest = results / f"payload-{args.corpus}-k{args.limit}-{time.strftime('%Y%m%d-%H%M%S')}.json"
    dest.write_text(json.dumps(out, indent=2, default=list))
    print(json.dumps(out["summary"], indent=2))
    print(f"wrote {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
