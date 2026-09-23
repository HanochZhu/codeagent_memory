#!/usr/bin/env python3
"""Evaluate cam retrieval on the cleaned LongMemEval-S dataset."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from eval_paths import cam_binary, dataset  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
DATA_URL = (
    "https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/"
    "resolve/main/longmemeval_s_cleaned.json"
)
DEFAULT_DATA = dataset("longmemeval")
PRINT_LOCK = threading.Lock()


def cam_bin() -> Path:
    return cam_binary()


def download(url: str, dest: Path) -> None:
    if dest.exists() and dest.stat().st_size > 1_000_000:
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    partial = dest.with_suffix(f"{dest.suffix}.partial")
    req = urllib.request.Request(url, headers={"User-Agent": "cam-longmemeval"})
    with urllib.request.urlopen(req, timeout=120) as response, partial.open("wb") as out:
        while chunk := response.read(1024 * 1024):
            out.write(chunk)
    partial.replace(dest)


class CamMcp:
    def __init__(self, executable: Path, project: Path) -> None:
        env = {**os.environ, "CAM_HASH_EMBED": "1"}
        self.next_id = 1
        self.process = subprocess.Popen(
            [str(executable), "mcp", "--project", str(project)],
            cwd=project,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        try:
            self.request(
                "initialize",
                {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "cam-longmemeval", "version": "1"},
                },
            )
            self.notify("notifications/initialized")
        except BaseException:
            self.close()
            raise

    def close(self) -> None:
        if self.process.poll() is not None:
            return
        if self.process.stdin:
            try:
                self.process.stdin.close()
            except BrokenPipeError:
                pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()

    def __enter__(self) -> "CamMcp":
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()

    def notify(self, method: str, params: dict | None = None) -> None:
        self._write({"jsonrpc": "2.0", "method": method, "params": params or {}})

    def request(self, method: str, params: dict | None = None) -> dict:
        request_id = self.next_id
        self.next_id += 1
        self._write(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params or {},
            }
        )
        if not self.process.stdout:
            raise RuntimeError("cam MCP stdout is unavailable")
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError(f"cam MCP exited while handling {method}")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"cam MCP {method} failed: {response['error']}")
        return response["result"]

    def call(self, name: str, arguments: dict) -> dict | list:
        result = self.request(
            "tools/call", {"name": name, "arguments": arguments}
        )
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        return json.loads(result["content"][0]["text"])

    def _write(self, payload: dict) -> None:
        if not self.process.stdin:
            raise RuntimeError("cam MCP stdin is unavailable")
        self.process.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
        self.process.stdin.flush()


def session_body(session_id: str, date: str, turns: list[dict]) -> str:
    lines = [f"SESSION_ID: {session_id}", f"DATE: {date}"]
    lines.extend(
        f"{turn.get('role', 'unknown').upper()}: {turn.get('content', '')}"
        for turn in turns
    )
    return "\n".join(lines)


def session_id_from_hit(hit: dict) -> str | None:
    for line in (hit.get("body") or "").splitlines():
        if line.startswith("SESSION_ID: "):
            return line.removeprefix("SESSION_ID: ").strip()
    return None


def percentile(values: list[float], p: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, round(p * (len(ordered) - 1)))
    return ordered[index]


def evaluate_instance(
    executable: Path,
    instance: dict,
    index: int,
    total: int,
    k: int,
    fusion: str,
) -> dict:
    started = time.perf_counter()
    with tempfile.TemporaryDirectory(prefix="cam-longmemeval-") as tmp:
        with CamMcp(executable, Path(tmp)) as cam:
            for session_id, date, turns in zip(
                instance["haystack_session_ids"],
                instance["haystack_dates"],
                instance["haystack_sessions"],
                strict=True,
            ):
                cam.call(
                    "cam_add",
                    {
                        "summary": f"Session {session_id} at {date}",
                        "body": session_body(session_id, date, turns),
                        "hash_embed": True,
                    },
                )
            ingestion_ms = (time.perf_counter() - started) * 1000
            recall_started = time.perf_counter()
            hits = cam.call(
                "cam_recall",
                {
                    "query": instance["question"],
                    "limit": k,
                    "fusion": fusion,
                    "hash_embed": True,
                },
            )
            recall_ms = (time.perf_counter() - recall_started) * 1000

    ranked = []
    for hit in hits:
        session_id = session_id_from_hit(hit)
        if session_id and session_id not in ranked:
            ranked.append(session_id)
    gold = set(instance["answer_session_ids"])
    row = {
        "question_id": instance["question_id"],
        "question_type": instance["question_type"],
        "gold": sorted(gold),
        "ranked": ranked,
        "recall_ms": recall_ms,
        "ingestion_ms": ingestion_ms,
        "gold_rank": next(
            (rank for rank, session_id in enumerate(ranked, 1) if session_id in gold),
            None,
        ),
    }
    for cutoff in (1, 5, 10):
        if cutoff <= k:
            found = len(gold.intersection(ranked[:cutoff]))
            row[f"recall@{cutoff}"] = found / len(gold)
            row[f"hit@{cutoff}"] = found > 0
    with PRINT_LOCK:
        print(
            f"[{index:03d}/{total:03d}] {row['question_id']} "
            f"rank={row['gold_rank']} latency={recall_ms:.1f}ms",
            flush=True,
        )
    return row


def aggregate(rows: list[dict], args: argparse.Namespace, data_sha256: str) -> dict:
    summary = {
        "dataset": "LongMemEval-S cleaned",
        "dataset_sha256": data_sha256,
        "n": len(rows),
        "sample_seed": args.seed,
        "k": args.k,
        "adapter": "cam",
        "embedder": "hash",
        "fusion": args.fusion,
        "ingestion": "one raw session per memory",
        "MRR": statistics.mean(
            1 / row["gold_rank"] if row["gold_rank"] else 0 for row in rows
        ),
        "recall_p50_ms": percentile([row["recall_ms"] for row in rows], 0.50),
        "recall_p95_ms": percentile([row["recall_ms"] for row in rows], 0.95),
        "mean_ingestion_ms": statistics.mean(row["ingestion_ms"] for row in rows),
        "by_type": {},
    }
    for cutoff in (1, 5, 10):
        if cutoff <= args.k:
            summary[f"R@{cutoff}"] = statistics.mean(
                row[f"recall@{cutoff}"] for row in rows
            )
            summary[f"Hit@{cutoff}"] = statistics.mean(
                row[f"hit@{cutoff}"] for row in rows
            )
    grouped: dict[str, list[dict]] = {}
    for row in rows:
        grouped.setdefault(row["question_type"], []).append(row)
    for question_type, group in sorted(grouped.items()):
        type_summary = {"n": len(group)}
        for cutoff in (1, 5, 10):
            if cutoff <= args.k:
                type_summary[f"R@{cutoff}"] = statistics.mean(
                    row[f"recall@{cutoff}"] for row in group
                )
        summary["by_type"][question_type] = type_summary
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, default=DEFAULT_DATA)
    parser.add_argument("--sample", type=int, default=100)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--k", type=int, default=10)
    parser.add_argument("--fusion", choices=("rrf", "sum"), default="rrf")
    parser.add_argument("--workers", type=int, default=4)
    args = parser.parse_args()

    download(DATA_URL, args.data)
    data_bytes = args.data.read_bytes()
    dataset = json.loads(data_bytes)
    eligible = [
        instance
        for instance in dataset
        if instance["answer_session_ids"]
        and not instance["question_id"].endswith("_abs")
    ]
    if args.sample > len(eligible):
        parser.error(f"--sample must be <= {len(eligible)}")
    selected = random.Random(args.seed).sample(eligible, args.sample)
    executable = cam_bin()

    rows = []
    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        futures = {
            executor.submit(
                evaluate_instance,
                executable,
                instance,
                index,
                len(selected),
                args.k,
                args.fusion,
            ): index
            for index, instance in enumerate(selected, 1)
        }
        for future in as_completed(futures):
            rows.append(future.result())
    rows.sort(key=lambda row: row["question_id"])

    summary = aggregate(rows, args, hashlib.sha256(data_bytes).hexdigest())
    output = {"summary": summary, "rows": rows}
    output_dir = HERE / "results"
    output_dir.mkdir(exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    output_path = output_dir / f"cam-longmemeval-s-{stamp}.json"
    output_path.write_text(json.dumps(output, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    print(f"result: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
