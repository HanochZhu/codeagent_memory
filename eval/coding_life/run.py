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

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from eval_paths import dataset  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
DEFAULT_DATA = dataset("coding_life")
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


def cam_json(
    cam: Path,
    root: Path,
    args: list[str],
    stdin: str | None = None,
    env: dict | None = None,
) -> dict | list:
    cmd = [str(cam), "--json", "--project", str(root), *args]
    proc = subprocess.run(
        cmd,
        cwd=root,
        input=stdin,
        text=True,
        capture_output=True,
        check=False,
        env=env,
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


def session_ranking(hits: list[dict]) -> list[str]:
    """Node hits mapped to session ids, first occurrence kept. A chained corpus
    holds several memories per session, and scoring is per session, so counting
    the same session twice would push recall above 1.0.
    """
    ranked: list[str] = []
    for hit in hits:
        sid = session_id_from_hit(hit) or ""
        if sid not in ranked:
            ranked.append(sid)
    return ranked


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


def score_lineage(
    gold: list[str], hits: list[dict], chains: dict[str, list[str]], k: int
) -> dict:
    """Did the top-k carry the newest node of each gold chain, and was anything
    older than its own chain tail returned flagged `latest`?"""
    top = hits[:k]
    top_ids = {h["id"] for h in top}
    tails = [chains[g][-1] for g in gold if g in chains]
    tail_ids = {chain[-1] for chain in chains.values()}
    return {
        "newestRecallAtK": (
            sum(1 for t in tails if t in top_ids) / len(tails) if tails else 0.0
        ),
        "staleLatest": sum(
            1 for h in top if h.get("latest") and h["id"] not in tail_ids
        ),
    }


def turns(sess: dict) -> list[str]:
    return [line for line in (x.strip() for x in sess["content"].splitlines()) if line]


def ingest_flat(cam, root, sessions, embed_flag, env) -> dict[str, list[str]]:
    for sess in sessions:
        body = f"{sess['id']}\n{sess.get('timestamp') or ''}\n{sess['content']}"
        summary = f"{sess['id']}: {sess['content'].splitlines()[0][:80]}"
        cam_json(
            cam,
            root,
            ["add", "--summary", summary, *embed_flag],
            stdin=body,
            env=env,
        )
    return {}


def ingest_lineage(cam, root, sessions, embed_flag, env) -> dict[str, list[str]]:
    """One memory per conversation turn, each turn a `--parent` revision of the
    previous one — the update model cam documents.
    """
    by_session = {sess["id"]: turns(sess) for sess in sessions}
    chains: dict[str, list[str]] = {}
    depth = 0
    while True:
        level = [s for s in sessions if depth < len(by_session[s["id"]])]
        if not level:
            return chains
        for sess in level:
            sid = sess["id"]
            line = by_session[sid][depth]
            head = sid if depth == 0 else f"{sid} rev{depth}"
            args = ["add", "--summary", f"{head}: {line[:80]}", *embed_flag]
            if depth:
                args += ["--parent", chains[sid][-1]]
            added = cam_json(
                cam,
                root,
                args,
                stdin=f"{sid}\n{sess.get('timestamp') or ''}\n{line}",
                env=env,
            )
            chains.setdefault(sid, []).append(added["id"])
        depth += 1


def tokenize_grep(text: str) -> list[str]:
    return [t for t in re.sub(r"[^a-z0-9_]+", " ", text.lower()).split() if len(t) > 2]


def grep_rank(question: str, sessions: list[dict], k: int) -> list[str]:
    terms = tokenize_grep(question)
    scored: list[tuple[str, int]] = []
    for sess in sessions:
        body = sess["content"].lower()
        hits = sum(1 for t in terms if t in body)
        if hits:
            scored.append((sess["id"], hits))
    scored.sort(key=lambda x: (-x[1], x[0]))
    return [sid for sid, _ in scored[:k]]


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--data", type=Path, default=DEFAULT_DATA)
    p.add_argument("--k", type=int, default=5)
    p.add_argument(
        "--hash-embed",
        action="store_true",
        help="use the test hash embedder (no model2vec download)",
    )
    p.add_argument(
        "--fusion",
        choices=("sum", "rrf"),
        default="rrf",
        help="solution score fusion: RRF k=60 (default) or min-max sum",
    )
    p.add_argument(
        "--adapter",
        choices=("cam", "grep"),
        default="cam",
        help="cam recall (default) or tokenized-substring grep baseline",
    )
    p.add_argument(
        "--lineage",
        action="store_true",
        help="ingest each conversation turn as a --parent revision instead of "
        "one flat memory per session",
    )
    p.add_argument(
        "--no-expand",
        action="store_true",
        help="turn off lineage expansion in recall, for A/B against --lineage",
    )
    args = p.parse_args()
    sessions = json.loads((args.data / "sessions.json").read_text())
    queries = json.loads((args.data / "queries.json").read_text())

    rows = []
    if args.adapter == "grep":
        for q in queries:
            t0 = time.perf_counter()
            ranked = grep_rank(q["question"], sessions, args.k)
            latency = (time.perf_counter() - t0) * 1000
            scored = score_question(q["goldSessionIds"], ranked, args.k)
            scored.update({"id": q["id"], "type": q["type"], "latency_ms": latency})
            rows.append(scored)
    else:
        cam = cam_bin()
        embed_flag = ["--hash-embed"] if args.hash_embed else []
        env = os.environ.copy()
        if not args.hash_embed:
            env["CAM_REQUIRE_MODEL2VEC"] = "1"
        if args.no_expand:
            env["CAM_NO_EXPAND"] = "1"

        with tempfile.TemporaryDirectory(prefix="cam-life-") as tmp:
            root = Path(tmp)
            ingest = ingest_lineage if args.lineage else ingest_flat
            chains = ingest(cam, root, sessions, embed_flag, env)

            for q in queries:
                t0 = time.perf_counter()
                hits = cam_json(
                    cam,
                    root,
                    [
                        "recall",
                        q["question"],
                        "--limit",
                        str(args.k),
                        "--fusion",
                        args.fusion,
                        *embed_flag,
                    ],
                    env=env,
                )
                latency = (time.perf_counter() - t0) * 1000
                ranked = session_ranking(hits)
                scored = score_question(q["goldSessionIds"], ranked, args.k)
                if chains:
                    scored.update(
                        score_lineage(q["goldSessionIds"], hits, chains, args.k)
                    )
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
        "adapter": args.adapter,
        "hash_embed": args.hash_embed,
        "embedder": (
            "n/a"
            if args.adapter == "grep"
            else ("hash" if args.hash_embed else "potion-multilingual-128M")
        ),
        "fusion": None if args.adapter == "grep" else args.fusion,
        "corpus": "lineage" if args.lineage else "flat",
        "expand": args.adapter == "cam" and not args.no_expand,
        "P@k": sum(r["precisionAtK"] for r in rows) / n,
        "R@k": sum(r["recallAtK"] for r in rows) / n,
        "hit_rate": sum(1 for r in rows if r["hit"]) / n,
        "p50_ms": sorted(r["latency_ms"] for r in rows)[n // 2],
        "ceiling_P@5": 0.240,
        "by_type": {},
        "misses": [r["id"] for r in rows if not r["hit"]],
        "partial_misses": [r["id"] for r in rows if r["recallAtK"] < 1.0],
    }
    if any("newestRecallAtK" in r for r in rows):
        summary["newest_R@k"] = sum(r["newestRecallAtK"] for r in rows) / n
        summary["stale_latest"] = sum(r["staleLatest"] for r in rows)
        summary["newest_misses"] = [
            r["id"] for r in rows if r["newestRecallAtK"] < 1.0
        ]

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
    tag = "grep" if args.adapter == "grep" else f"cam-{args.fusion}"
    if args.lineage:
        tag += "-lineage" if summary["expand"] else "-lineage-noexpand"
    (out_dir / f"cam-life-{tag}-{stamp}.json").write_text(
        json.dumps({"summary": summary, "rows": rows}, indent=2)
    )
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
