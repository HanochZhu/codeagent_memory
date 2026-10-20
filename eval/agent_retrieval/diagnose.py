#!/usr/bin/env python3
"""Failure breakdown and post-hoc rank fusion for edit2ripple.

Reads a finished `run.py` result plus the official `arb` baseline details,
then reports how far the cam graph can reach from the anchor file, which gold
files each ranker finds, and what unweighted RRF over the stored top-20 lists
would score. Benchmark evidence labels are used only to slice the report.
"""

from __future__ import annotations

import argparse
import itertools
import json
import sqlite3
import statistics
from collections import Counter
from pathlib import Path

from agent_retrieval_bench.bcy_curve import CorpusFileCache, pack_files
from agent_retrieval_bench.rank_fusion import file_rank_metrics, reciprocal_rank_fusion

from run import DEFAULT_BENCHMARK, DEFAULT_CORPUS, HERE, corpus_manifest, read_jsonl

GRAPH_EDGES = ("calls", "references", "implements")
DEPTH = 20
RRF_K = 60


def latest_cam_result() -> Path:
    results = sorted((HERE / "results").glob("cam-edit2ripple-*.json"))
    if not results:
        raise SystemExit("no cam-edit2ripple result; run eval/agent_retrieval/run.py first")
    return results[-1]


def load_details(path: Path) -> dict[str, list[str]]:
    return {row["sample_id"]: row["top_files"][:DEPTH] for row in read_jsonl(path)}


def graph_reach(db_path: Path, anchor_file: str) -> tuple[int, set[str], set[str], set[str]]:
    """Anchor symbol count and the files one and two graph hops away."""
    with sqlite3.connect(db_path) as connection:
        indexed = {row[0] for row in connection.execute("SELECT path FROM files")}
        seeds = {
            row[0]
            for row in connection.execute(
                "SELECT id FROM nodes WHERE file_path = ? AND kind != 'file'", (anchor_file,)
            )
        }
        kinds = ",".join("?" * len(GRAPH_EDGES))

        def neighbours(ids: set[str]) -> set[str]:
            found: set[str] = set()
            for node_id in ids:
                for (other,) in connection.execute(
                    f"""
                    SELECT CASE WHEN source = ? THEN target ELSE source END
                    FROM edges
                    WHERE (source = ? OR target = ?) AND kind IN ({kinds})
                    """,
                    (node_id, node_id, node_id, *GRAPH_EDGES),
                ):
                    found.add(other)
            return found

        def files_of(ids: set[str]) -> set[str]:
            paths: set[str] = set()
            for node_id in ids:
                row = connection.execute(
                    "SELECT file_path FROM nodes WHERE id = ?", (node_id,)
                ).fetchone()
                if row:
                    paths.add(row[0])
            return paths

        hop1 = neighbours(seeds)
        hop2 = neighbours(hop1 - seeds) | hop1
        return len(seeds), indexed, files_of(hop1), files_of(hop2)


def rank_in(ranked: list[str], path: str) -> int | None:
    try:
        return ranked.index(path) + 1
    except ValueError:
        return None


def summarize(rows: list[dict]) -> dict[str, float]:
    keys = ("Recall@5", "Recall@10", "Recall@20", "MRR", "BCY@8k")
    return {key: statistics.mean(row[key] for row in rows) for key in keys}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--benchmark", type=Path, default=DEFAULT_BENCHMARK)
    parser.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    parser.add_argument("--cam", type=Path, default=None, help="run.py result JSON")
    parser.add_argument("--baselines", type=Path, default=HERE / "results" / "baselines")
    args = parser.parse_args()

    cam_path = args.cam or latest_cam_result()
    cam_rows = {row["sample_id"]: row for row in json.loads(cam_path.read_text())["rows"]}
    rankers = {"cam": {sid: row["top_files"][:DEPTH] for sid, row in cam_rows.items()}}
    for name in ("lexical", "repomap", "bm25"):
        details = args.baselines / f"{name}_details.jsonl"
        if details.exists():
            rankers[name] = load_details(details)

    samples = read_jsonl(args.benchmark / "samples.jsonl")
    manifest = corpus_manifest(args.corpus)
    cache = CorpusFileCache(
        {(s["repo"], s["base_commit"]): manifest[(s["repo"], s["base_commit"])] for s in samples}
    )
    corpora = HERE / "data" / "cam_corpora"

    gold_rows = []
    sample_rows = []
    for sample in samples:
        sid = sample["id"]
        anchor = sample["query"]["anchor_file"]
        db = corpora / sample["repo"].replace("/", "__") / sample["base_commit"] / ".cam" / "cam.db"
        seed_count, indexed, hop1, hop2 = graph_reach(db, anchor)
        evidence = {e["path"]: e for e in sample["metadata"].get("gold_evidence", [])}
        sample_rows.append(
            {
                "sample_id": sid,
                "anchor": anchor,
                "anchor_symbols": seed_count,
                "hop1_files": len(hop1 - {anchor}),
                "hop2_files": len(hop2 - {anchor}),
            }
        )
        for path in sample["gold"]["files"]:
            info = evidence.get(path, {})
            gold_rows.append(
                {
                    "sample_id": sid,
                    "repo": sample["repo"],
                    "path": path,
                    "is_test": bool(info.get("is_test")),
                    "signals": info.get("signals", []),
                    "indexed": path in indexed,
                    "hop1": path in hop1,
                    "hop2": path in hop2,
                    "anchor_symbols": seed_count,
                    **{f"rank_{name}": rank_in(ranked.get(sid, []), path) for name, ranked in rankers.items()},
                }
            )

    def scored(ranked_by_sample: dict[str, list[str]]) -> list[dict]:
        rows = []
        for sample in samples:
            ranked = ranked_by_sample.get(sample["id"], [])[:DEPTH]
            gold = sample["gold"]["files"]
            metrics = file_rank_metrics(gold, ranked)
            metrics["BCY@8k"] = pack_files(
                sample["repo"], sample["base_commit"], ranked, set(gold), cache, 8_000
            )["bcy"]
            rows.append(metrics)
        return rows

    leaderboard = {name: summarize(scored(ranked)) for name, ranked in rankers.items()}
    names = list(rankers)
    for size in range(2, len(names) + 1):
        for combo in itertools.combinations(names, size):
            fused = {
                sample["id"]: reciprocal_rank_fusion(
                    [rankers[name].get(sample["id"], []) for name in combo], rrf_k=RRF_K
                )
                for sample in samples
            }
            leaderboard["rrf(" + "+".join(combo) + ")"] = summarize(scored(fused))

    total = len(gold_rows)

    def share(predicate) -> float:
        return sum(1 for row in gold_rows if predicate(row)) / total

    reach = {
        "gold_files": total,
        "samples_without_anchor_symbols": sum(1 for row in sample_rows if row["anchor_symbols"] == 0),
        "gold_indexed": share(lambda r: r["indexed"]),
        "gold_in_hop1": share(lambda r: r["hop1"]),
        "gold_in_hop2": share(lambda r: r["hop2"]),
        "gold_in_cam_top20": share(lambda r: r["rank_cam"] is not None),
        "hop1_but_ranked_out_of_top20": share(lambda r: r["hop1"] and r["rank_cam"] is None),
        "hop1_files_median": statistics.median(row["hop1_files"] for row in sample_rows),
        "hop2_files_median": statistics.median(row["hop2_files"] for row in sample_rows),
        "hop2_files_p90": statistics.quantiles([row["hop2_files"] for row in sample_rows], n=10)[-1],
    }

    def slice_table(key_of) -> dict[str, dict[str, float]]:
        groups: dict[str, list[dict]] = {}
        for row in gold_rows:
            for key in key_of(row):
                groups.setdefault(key, []).append(row)
        table = {}
        for key, rows in sorted(groups.items()):
            entry = {
                "n": len(rows),
                "indexed": statistics.mean(r["indexed"] for r in rows),
                "hop1": statistics.mean(r["hop1"] for r in rows),
                "hop2": statistics.mean(r["hop2"] for r in rows),
            }
            for name in rankers:
                entry[f"{name}@20"] = statistics.mean(r[f"rank_{name}"] is not None for r in rows)
                entry[f"{name}@5"] = statistics.mean(
                    r[f"rank_{name}"] is not None and r[f"rank_{name}"] <= 5 for r in rows
                )
            table[key] = entry
        return table

    by_kind = slice_table(lambda r: ["test" if r["is_test"] else "source"])
    by_signal = slice_table(lambda r: r["signals"] or ["(none)"])

    overlap = Counter()
    for row in gold_rows:
        hits = tuple(name for name in rankers if row[f"rank_{name}"] is not None)
        overlap[" & ".join(hits) or "(none)"] += 1

    report = {
        "cam_result": str(cam_path),
        "reach": reach,
        "leaderboard": leaderboard,
        "by_gold_kind": by_kind,
        "by_evidence_signal": by_signal,
        "top20_overlap": dict(overlap.most_common()),
    }
    out = HERE / "results" / f"diagnose-{cam_path.stem.removeprefix('cam-edit2ripple-')}.json"
    out.write_text(json.dumps({"summary": report, "gold": gold_rows}, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))
    print(f"result: {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
