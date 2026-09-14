#!/usr/bin/env python3
"""coding-agent-life-v1 via cam add / recall.

Ingest each session as a solution, query with cam recall, score R@5 / hit rate
the same way as agentmemory/eval/runner/score.ts.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
DEFAULT_DATA = Path(
    "/Users/moira/Documents/code/AI/memory/agentmemory/eval/data/coding-agent-life-v1"
)
SESS_RE = re.compile(r"sess-\d+")


def cam_bin() -> Path:
    env = os.environ.get("CAM_BIN")
    if env:
        return Path(env)
    debug = REPO / "target" / "debug" / "cam"
    if debug.exists():
        return debug
    subprocess.check_call(["cargo", "build", "-q"], cwd=REPO)
    return debug


def cam_json(cam: Path, root: Path, args: list[str], stdin: str | None = None) -> dict | list:
    cmd = [str(cam), "--json", "--path", str(root), *args]
    proc = subprocess.run(
        cmd,
        cwd=root,
        input=stdin,
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}\n{proc.stderr}")
    return json.loads(proc.stdout)


def session_id_from_hit(hit: dict) -> str | None:
    for field in (hit.get("summary") or "", hit.get("body") or ""):
        m = SESS_RE.search(field)
        if m:
            return m.group(0)
    return None


def score_question(gold: list[str], ranked: list[str], k: int) -> dict:
    top = ranked[:k]
    gset = set(gold)
    hits = sum(1 for i in top if i in gset)
    return {
        "precisionAtK": hits / k if k else 0.0,
        "recallAtK": hits / len(gset) if gset else 0.0,
        "hit": hits > 0,
        "topGoldRank": next((i + 1 for i, s in enumerate(ranked) if s in gset), None),
        "ranked": top,
    }


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--data", type=Path, default=DEFAULT_DATA)
    p.add_argument("--k", type=int, default=5)
    p.add_argument(
        "--hash-embed",
        action="store_true",
        help="use the test hash embedder (no model2vec download)",
    )
    args = p.parse_args()
    sessions = json.loads((args.data / "sessions.json").read_text())
    queries = json.loads((args.data / "queries.json").read_text())
    cam = cam_bin()
    embed_flag = ["--hash-embed"] if args.hash_embed else []

    with tempfile.TemporaryDirectory(prefix="cam-life-") as tmp:
        root = Path(tmp)
        cam_json(cam, root, ["init"])
        for sess in sessions:
            body = f"{sess['id']}\n{sess.get('timestamp') or ''}\n{sess['content']}"
            summary = f"{sess['id']}: {sess['content'].splitlines()[0][:80]}"
            cam_json(
                cam,
                root,
                ["add", "--summary", summary, *embed_flag],
                stdin=body,
            )

        rows = []
        for q in queries:
            t0 = time.perf_counter()
            hits = cam_json(
                cam,
                root,
                ["recall", q["question"], "--limit", str(args.k), *embed_flag],
            )
            latency = (time.perf_counter() - t0) * 1000
            ranked = [session_id_from_hit(h) or "" for h in hits]
            scored = score_question(q["goldSessionIds"], ranked, args.k)
            scored.update(
                {
                    "id": q["id"],
                    "type": q["type"],
                    "latency_ms": latency,
                }
            )
            rows.append(scored)

    n = len(rows)
    summary = {
        "n": n,
        "k": args.k,
        "hash_embed": args.hash_embed,
        "P@k": sum(r["precisionAtK"] for r in rows) / n,
        "R@k": sum(r["recallAtK"] for r in rows) / n,
        "hit_rate": sum(1 for r in rows if r["hit"]) / n,
        "p50_ms": sorted(r["latency_ms"] for r in rows)[n // 2],
        "ceiling_P@5": 0.240,
        "by_type": {},
        "misses": [r["id"] for r in rows if not r["hit"]],
    }
    by: dict[str, list] = {}
    for r in rows:
        by.setdefault(r["type"], []).append(r)
    summary["by_type"] = {
        t: {
            "n": len(xs),
            "P@k": sum(x["precisionAtK"] for x in xs) / len(xs),
            "R@k": sum(x["recallAtK"] for x in xs) / len(xs),
            "hit_rate": sum(1 for x in xs if x["hit"]) / len(xs),
        }
        for t, xs in by.items()
    }
    out_dir = HERE / "results"
    out_dir.mkdir(exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    (out_dir / f"cam-life-{stamp}.json").write_text(
        json.dumps({"summary": summary, "rows": rows}, indent=2)
    )
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
