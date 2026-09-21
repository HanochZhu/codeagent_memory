#!/usr/bin/env python3
"""Fetch a LongMemCode scenario file, index with cam or codegraph, score.

Example:
  python3 eval/longmemcode/run.py --corpus clap --limit 50
  python3 eval/longmemcode/run.py --corpus clap --backend codegraph
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
SCENARIO_URL = "https://raw.githubusercontent.com/CataDef/LongMemCode/main/scenarios/{name}.json"

CORPORA = {
    "clap": {
        "git": "https://github.com/clap-rs/clap.git",
        "tag": "v4.6.1",
        "scenarios": "clap",
    },
    "fastapi": {
        "git": "https://github.com/fastapi/fastapi.git",
        "tag": "0.115.6",
        "scenarios": "fastapi",
    },
}


def download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    req = urllib.request.Request(url, headers={"User-Agent": "cam-longmemcode"})
    # Some macOS Pythons lack certifi; curl is more reliable here.
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            dest.write_bytes(resp.read())
            return
    except Exception:
        pass
    subprocess.check_call(["curl", "-4", "-sL", "--max-time", "60", "-o", str(dest), url])


def ensure_scenarios(name: str) -> Path:
    path = HERE / "data" / f"{name}.json"
    if not path.exists() or path.stat().st_size < 1000:
        download(SCENARIO_URL.format(name=name), path)
    return path


def ensure_corpus(name: str) -> Path:
    spec = CORPORA[name]
    root = HERE / "corpora" / name
    if not (root / ".git").exists():
        root.parent.mkdir(parents=True, exist_ok=True)
        subprocess.check_call(
            [
                "git",
                "clone",
                "--depth",
                "1",
                "--branch",
                spec["tag"],
                spec["git"],
                str(root),
            ]
        )
    return root


def cam_bin() -> Path:
    env = os.environ.get("CAM_BIN")
    if env:
        return Path(env)
    debug = REPO / "target" / "debug" / "cam"
    cached = Path.home() / ".cargo" / "bin" / "cam"
    if debug.exists():
        return debug
    if cached.exists():
        return cached
    subprocess.check_call(
        ["cargo", "build", "-q"],
        cwd=REPO,
        env={**os.environ, "PATH": os.environ.get("PATH", "")},
    )
    return debug


def codegraph_bin() -> str:
    env = os.environ.get("CODEGRAPH_BIN")
    if env:
        return env
    return "codegraph"


def index_corpus(backend: str, corpus: Path) -> None:
    if backend == "codegraph":
        bin_ = codegraph_bin()
        marker = corpus / ".codegraph"
        if not marker.exists():
            subprocess.check_call([bin_, "init", "-y", str(corpus)], cwd=corpus)
        else:
            subprocess.check_call([bin_, "index", str(corpus)], cwd=corpus)
        return
    cam = cam_bin()
    subprocess.check_call([str(cam), "--path", str(corpus), "index"], cwd=corpus)


def graph_db(backend: str, corpus: Path) -> Path:
    if backend == "codegraph":
        return corpus / ".codegraph" / "codegraph.db"
    return corpus / ".cam" / "cam.db"


def main() -> int:
    sys.path.insert(0, str(HERE))
    from adapter import answer, load_catalog
    from graph import CamGraph
    from score import ONE_HOP_SKIP_SUBTYPES, score_expected, summarize

    p = argparse.ArgumentParser()
    p.add_argument("--corpus", default="clap", choices=sorted(CORPORA))
    p.add_argument("--backend", default="cam", choices=("cam", "codegraph"))
    p.add_argument("--limit", type=int, default=0, help="score first N scenarios (0=all)")
    p.add_argument("--skip-index", action="store_true")
    p.add_argument("--source", default="", help="already-checked-out corpus root")
    args = p.parse_args()

    spec = CORPORA[args.corpus]
    scenarios_path = ensure_scenarios(spec["scenarios"])
    scenarios = json.loads(scenarios_path.read_text())
    if args.limit:
        scenarios = scenarios[: args.limit]

    source = Path(args.source) if args.source else ensure_corpus(args.corpus)
    if not args.skip_index:
        index_corpus(args.backend, source)

    db = graph_db(args.backend, source)
    if not db.exists():
        print(f"error: {db} missing after index", file=sys.stderr)
        return 1

    catalog = load_catalog(scenarios_path)
    if args.backend == "codegraph":
        from codegraph_graph import CodegraphGraph

        graph = CodegraphGraph(db, source)
    else:
        graph = CamGraph(db)
    rows = []
    t0 = time.perf_counter()
    latencies = []
    for sc in scenarios:
        q = sc["query"]
        start = time.perf_counter()
        results = answer(q, graph, catalog)
        latencies.append(time.perf_counter() - start)
        rows.append(
            {
                "id": sc["id"],
                "category": sc["category"],
                "sub_type": sc["sub_type"],
                "op": q.get("op"),
                "gold_source": sc.get("gold_source"),
                "score": score_expected(sc["expected"], results),
                "n_returned": len(results),
                "deferred": sc["sub_type"] in ONE_HOP_SKIP_SUBTYPES,
            }
        )
    elapsed = time.perf_counter() - t0
    latencies_sorted = sorted(latencies)
    def pct(p: float) -> float:
        if not latencies_sorted:
            return 0.0
        i = min(len(latencies_sorted) - 1, int(round(p * (len(latencies_sorted) - 1))))
        return latencies_sorted[i] * 1000

    summary = summarize(rows)
    summary.update(
        {
            "corpus": args.corpus,
            "backend": args.backend,
            "source": str(source),
            "elapsed_s": elapsed,
            "p50_ms": pct(0.50),
            "p95_ms": pct(0.95),
            "p99_ms": pct(0.99),
            "cost_usd_per_1k": 0.0,
        }
    )
    out_dir = HERE / "results"
    out_dir.mkdir(exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    (out_dir / f"{args.backend}-{args.corpus}-{stamp}.json").write_text(
        json.dumps({"summary": summary, "scenarios": rows}, indent=2)
    )
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
