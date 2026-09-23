#!/usr/bin/env python3
"""Graph-only cam adapter for Agent Retrieval Bench V2 edit2ripple."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sqlite3
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path, PurePosixPath

from agent_retrieval_bench.baseline import (
    gold_file_ranks,
    sample_metrics,
    summarize_details,
    target_gold_files,
)
from agent_retrieval_bench.bcy_curve import CorpusFileCache, pack_files


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from eval_paths import cam_binary, dataset  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
DEFAULT_BENCHMARK = dataset("agent_retrieval_benchmark")
DEFAULT_CORPUS = dataset("agent_retrieval_corpus")
SUPPORTED_EXTENSIONS = {".go", ".js", ".jsx", ".py", ".rs", ".ts", ".tsx"}


def read_jsonl(path: Path) -> list[dict]:
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def cam_bin() -> Path:
    return cam_binary()


def corpus_manifest(corpus_dir: Path) -> dict[tuple[str, str], Path]:
    manifest = {}
    for row in read_jsonl(corpus_dir / "corpus_manifest.jsonl"):
        if row.get("status") != "ok":
            continue
        chunks = corpus_dir / Path(row["chunks_path"]).name
        if not chunks.exists():
            chunks = corpus_dir / row["repo"].replace("/", "__") / Path(
                row["chunks_path"]
            ).name
        manifest[(row["repo"], row["base_commit"])] = chunks
    return manifest


def file_texts(chunks_path: Path) -> dict[str, str]:
    files = {}
    for row in read_jsonl(chunks_path):
        if row.get("kind") == "file" and row.get("path"):
            files[row["path"]] = row.get("text") or ""
    return files


def safe_relative_path(value: str) -> Path | None:
    if not value or "\\" in value:
        return None
    posix = PurePosixPath(value)
    if posix.is_absolute() or any(":" in part or part == ".." for part in posix.parts):
        return None
    if os.name == "nt" and any(
        part.endswith((" ", ".")) or Path(part).is_reserved() for part in posix.parts
    ):
        return None
    return Path(*posix.parts)


def materialize_and_index(
    executable: Path,
    cache_root: Path,
    repo: str,
    base_commit: str,
    chunks_path: Path,
    force: bool,
) -> Path:
    root = cache_root / repo.replace("/", "__") / base_commit
    marker = root / ".cam" / "cam.db"
    if marker.exists() and not force:
        return root
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    for path, text in file_texts(chunks_path).items():
        relative = safe_relative_path(path)
        if relative is None:
            continue
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text, encoding="utf-8")
    subprocess.run(
        [str(executable), "--project", str(root), "index"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return root


def rank_graph_neighbors(
    db_path: Path,
    anchor_file: str,
    anchor_diff: str,
) -> tuple[list[str], float]:
    started = time.perf_counter()
    with sqlite3.connect(db_path) as connection:
        nodes = connection.execute(
            "SELECT id, name FROM nodes WHERE file_path = ? AND kind != 'file'",
            (anchor_file,),
        ).fetchall()
        if not nodes:
            return [], (time.perf_counter() - started) * 1000
        changed_names = {
            name
            for _, name in nodes
            if re.search(rf"\b{re.escape(name)}\b", anchor_diff)
        }
        scores: dict[str, float] = {}
        for node_id, name in nodes:
            anchor_weight = 2.0 if name in changed_names else 1.0
            rows = connection.execute(
                """
                SELECT n.file_path, e.kind
                FROM edges e
                JOIN nodes n
                  ON n.id = CASE WHEN e.source = ? THEN e.target ELSE e.source END
                WHERE (e.source = ? OR e.target = ?)
                  AND e.kind IN ('calls', 'references', 'implements')
                """,
                (node_id, node_id, node_id),
            ).fetchall()
            for path, _kind in rows:
                if path and path != anchor_file:
                    scores[path] = scores.get(path, 0.0) + anchor_weight
    ranked = sorted(scores, key=lambda path: (-scores[path], path))
    latency_ms = (time.perf_counter() - started) * 1000
    return ranked[:20], latency_ms


def evaluate_sample(
    sample: dict,
    roots: dict[tuple[str, str], Path],
    files_by_corpus: dict[tuple[str, str], dict[str, str]],
    corpus_cache: CorpusFileCache,
) -> dict:
    key = (sample["repo"], sample["base_commit"])
    query = sample.get("query") or {}
    anchor_file = query.get("anchor_file") or ""
    ranked, latency_ms = rank_graph_neighbors(
        roots[key] / ".cam" / "cam.db",
        anchor_file,
        query.get("anchor_diff") or "",
    )
    gold = target_gold_files(sample)
    files = files_by_corpus[key]
    ranked_chunks = [
        {"path": path, "text": files.get(path, ""), "kind": "file"}
        for path in ranked
    ]
    metrics = sample_metrics(gold, ranked_chunks)
    packed = pack_files(
        sample["repo"],
        sample["base_commit"],
        ranked,
        set(gold),
        corpus_cache,
        8_000,
    )
    metrics["BCY@8k"] = packed["bcy"]
    return {
        "sample_id": sample["id"],
        "task_type": sample["task_type"],
        "repo": sample["repo"],
        "base_commit": sample["base_commit"],
        "anchor_file": anchor_file,
        "anchor_supported": Path(anchor_file).suffix.lower() in SUPPORTED_EXTENSIONS,
        "ranker": "cam-graph-neighbors",
        "gold_files": gold,
        "gold_ranks": gold_file_ranks(gold, ranked_chunks),
        "top_files": ranked,
        "query_latency_ms": latency_ms,
        "metrics": metrics,
    }


def percentile(values: list[float], p: float) -> float:
    ordered = sorted(values)
    if not ordered:
        return 0.0
    return ordered[min(len(ordered) - 1, round(p * (len(ordered) - 1)))]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--benchmark", type=Path, default=DEFAULT_BENCHMARK)
    parser.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--force-index", action="store_true")
    args = parser.parse_args()

    samples = read_jsonl(args.benchmark / "samples.jsonl")
    if args.limit:
        samples = samples[: args.limit]
    manifest = corpus_manifest(args.corpus)
    required_keys = sorted(
        {(sample["repo"], sample["base_commit"]) for sample in samples}
    )
    missing = set(required_keys) - manifest.keys()
    if missing:
        parser.error(f"missing corpus snapshots: {sorted(missing)}")

    executable = cam_bin()
    cache_root = HERE / "data" / "cam_corpora"
    roots: dict[tuple[str, str], Path] = {}
    index_started = time.perf_counter()
    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        futures = {
            executor.submit(
                materialize_and_index,
                executable,
                cache_root,
                repo,
                commit,
                manifest[(repo, commit)],
                args.force_index,
            ): (repo, commit)
            for repo, commit in required_keys
        }
        for future in as_completed(futures):
            key = futures[future]
            roots[key] = future.result()
            print(f"indexed {len(roots):02d}/{len(required_keys):02d} {key[0]}", flush=True)
    index_seconds = time.perf_counter() - index_started

    files_by_corpus = {key: file_texts(manifest[key]) for key in required_keys}
    official_manifest = {key: manifest[key] for key in required_keys}
    corpus_cache = CorpusFileCache(official_manifest)
    rows = [
        evaluate_sample(sample, roots, files_by_corpus, corpus_cache)
        for sample in samples
    ]
    metrics = summarize_details(rows)
    latencies = [row["query_latency_ms"] for row in rows]
    metrics["BCY@8k"] = statistics.mean(
        row["metrics"]["BCY@8k"] for row in rows
    )
    result = {
        "dataset": "Agent Retrieval Bench V2 edit2ripple",
        "ranker": "cam graph neighbors of symbols in the anchored file",
        "evaluated": len(rows),
        "supported_anchor_files": sum(row["anchor_supported"] for row in rows),
        "unsupported_anchor_files": sum(not row["anchor_supported"] for row in rows),
        "metrics": metrics,
        "runtime": {
            "index_seconds": index_seconds,
            "query_p50_ms": percentile(latencies, 0.50),
            "query_p95_ms": percentile(latencies, 0.95),
        },
    }

    output_dir = HERE / "results"
    output_dir.mkdir(exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    output_path = output_dir / f"cam-edit2ripple-{stamp}.json"
    output_path.write_text(
        json.dumps({"summary": result, "rows": rows}, indent=2),
        encoding="utf-8",
    )
    print(json.dumps(result, indent=2))
    print(f"result: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
