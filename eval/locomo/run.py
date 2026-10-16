#!/usr/bin/env python3
"""Score cam retrieval on LoCoMo evidence dialogs.

Each dialog turn is one memory. A question is correct to the extent its
annotated evidence ids (D1:3) appear in the top-k recall hits. This is
evidence recall, not the paper's answer F1, which needs a reader model.
"""

from __future__ import annotations

import argparse
import hashlib
import json
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

from eval_paths import (  # noqa: E402
    POTION_MODEL,
    add_hash_embed_argument,
    cam_binary,
    dataset,
    embed_env,
)

HERE = Path(__file__).resolve().parent
DATA_URL = (
    "https://raw.githubusercontent.com/snap-research/locomo/main/data/locomo10.json"
)
DEFAULT_DATA = dataset("locomo")
PRINT_LOCK = threading.Lock()
CATEGORIES = {
    1: "multi-hop",
    2: "temporal",
    3: "open-domain",
    4: "single-hop",
    5: "adversarial",
}


class CamMcp:
    def __init__(self, executable: Path, project: Path, hash_embed: bool) -> None:
        env = embed_env(hash_embed=hash_embed)
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
                    "clientInfo": {"name": "cam-locomo", "version": "1"},
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
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        return json.loads(result["content"][0]["text"])

    def _write(self, payload: dict) -> None:
        if not self.process.stdin:
            raise RuntimeError("cam MCP stdin is unavailable")
        self.process.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
        self.process.stdin.flush()


def download(url: str, dest: Path) -> None:
    if dest.exists() and dest.stat().st_size > 1_000_000:
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    partial = dest.with_suffix(f"{dest.suffix}.partial")
    req = urllib.request.Request(url, headers={"User-Agent": "cam-locomo"})
    with urllib.request.urlopen(req, timeout=120) as response, partial.open("wb") as out:
        while chunk := response.read(1024 * 1024):
            out.write(chunk)
    partial.replace(dest)


def session_indexes(conversation: dict) -> list[int]:
    indexes = []
    index = 1
    while f"session_{index}" in conversation:
        indexes.append(index)
        index += 1
    return indexes


def turn_body(conversation: dict, session_index: int, turn: dict) -> str:
    date = conversation.get(f"session_{session_index}_date_time") or ""
    lines = [
        f"DIA_ID: {turn['dia_id']}",
        f"SESSION: {session_index}",
        f"DATE: {date}",
        f"SPEAKER: {turn.get('speaker') or ''}",
        turn.get("text") or "",
    ]
    if turn.get("blip_caption"):
        lines.append(f"IMAGE: {turn['blip_caption']}")
    return "\n".join(lines)


def dia_id_from_hit(hit: dict) -> str | None:
    for line in (hit.get("body") or "").splitlines():
        if line.startswith("DIA_ID: "):
            return line.removeprefix("DIA_ID: ").strip()
    return None


def percentile(values: list[float], p: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, round(p * (len(ordered) - 1)))
    return ordered[index]


def evaluate_sample(
    executable: Path,
    sample: dict,
    k: int,
    hash_embed: bool,
) -> list[dict]:
    conversation = sample["conversation"]
    started = time.perf_counter()
    rows = []
    with tempfile.TemporaryDirectory(prefix="cam-locomo-") as tmp:
        with CamMcp(executable, Path(tmp), hash_embed) as cam:
            for session_index in session_indexes(conversation):
                for turn in conversation[f"session_{session_index}"]:
                    cam.call(
                        "cam_add",
                        {
                            "summary": (
                                f"Dialog {turn['dia_id']} by {turn.get('speaker') or 'speaker'}"
                            ),
                            "body": turn_body(conversation, session_index, turn),
                            "hash_embed": hash_embed,
                        },
                    )
            ingestion_ms = (time.perf_counter() - started) * 1000
            for question_index, qa in enumerate(sample["qa"]):
                evidence = [item for item in (qa.get("evidence") or []) if item]
                if not evidence:
                    continue
                recall_started = time.perf_counter()
                hits = cam.call(
                    "cam_recall",
                    {
                        "query": qa["question"],
                        "limit": k,
                        "hash_embed": hash_embed,
                    },
                )
                recall_ms = (time.perf_counter() - recall_started) * 1000
                ranked = []
                for hit in hits:
                    dia_id = dia_id_from_hit(hit)
                    if dia_id and dia_id not in ranked:
                        ranked.append(dia_id)
                gold = list(dict.fromkeys(evidence))
                gold_set = set(gold)
                row = {
                    "sample_id": sample["sample_id"],
                    "question_index": question_index,
                    "category": qa["category"],
                    "category_name": CATEGORIES.get(qa["category"], str(qa["category"])),
                    "gold": gold,
                    "ranked": ranked,
                    "recall_ms": recall_ms,
                    "ingestion_ms": ingestion_ms,
                    "gold_rank": next(
                        (rank for rank, dia_id in enumerate(ranked, 1) if dia_id in gold_set),
                        None,
                    ),
                }
                for cutoff in (5, 10):
                    if cutoff <= k:
                        found = len(gold_set.intersection(ranked[:cutoff]))
                        row[f"recall@{cutoff}"] = found / len(gold_set)
                        row[f"hit@{cutoff}"] = found > 0
                rows.append(row)
    with PRINT_LOCK:
        print(
            f"{sample['sample_id']} questions={len(rows)} "
            f"ingest={ingestion_ms / 1000:.1f}s",
            flush=True,
        )
    return rows


def aggregate(rows: list[dict], args: argparse.Namespace, data_sha256: str) -> dict:
    summary = {
        "dataset": "LoCoMo locomo10",
        "dataset_sha256": data_sha256,
        "n": len(rows),
        "k": args.k,
        "unit": "dialog turn",
        "metric": "evidence dia_id recall, not answer F1",
        "embedder": "hash" if args.hash_embed else POTION_MODEL,
        "MRR": statistics.mean(
            1 / row["gold_rank"] if row["gold_rank"] else 0 for row in rows
        ),
        "recall_p50_ms": percentile([row["recall_ms"] for row in rows], 0.50),
        "recall_p95_ms": percentile([row["recall_ms"] for row in rows], 0.95),
        "by_category": {},
    }
    for cutoff in (5, 10):
        if cutoff <= args.k:
            summary[f"R@{cutoff}"] = statistics.mean(row[f"recall@{cutoff}"] for row in rows)
            summary[f"Hit@{cutoff}"] = statistics.mean(row[f"hit@{cutoff}"] for row in rows)
    grouped: dict[str, list[dict]] = {}
    for row in rows:
        grouped.setdefault(row["category_name"], []).append(row)
    for name, group in sorted(grouped.items()):
        type_summary = {"n": len(group)}
        for cutoff in (5, 10):
            if cutoff <= args.k:
                type_summary[f"R@{cutoff}"] = statistics.mean(
                    row[f"recall@{cutoff}"] for row in group
                )
                type_summary[f"Hit@{cutoff}"] = statistics.mean(
                    row[f"hit@{cutoff}"] for row in group
                )
        type_summary["MRR"] = statistics.mean(
            1 / row["gold_rank"] if row["gold_rank"] else 0 for row in group
        )
        summary["by_category"][name] = type_summary
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, default=DEFAULT_DATA)
    parser.add_argument("--k", type=int, default=10)
    parser.add_argument("--workers", type=int, default=4)
    add_hash_embed_argument(parser)
    args = parser.parse_args()

    download(DATA_URL, args.data)
    data_bytes = args.data.read_bytes()
    samples = json.loads(data_bytes)
    executable = cam_binary()

    rows: list[dict] = []
    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        futures = [
            executor.submit(evaluate_sample, executable, sample, args.k, args.hash_embed)
            for sample in samples
        ]
        for future in as_completed(futures):
            rows.extend(future.result())
    rows.sort(key=lambda row: (row["sample_id"], row["question_index"]))

    summary = aggregate(rows, args, hashlib.sha256(data_bytes).hexdigest())
    output_dir = HERE / "results"
    output_dir.mkdir(exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    tag = "hash" if args.hash_embed else "potion"
    output_path = output_dir / f"cam-locomo-{tag}-{stamp}.json"
    output_path.write_text(
        json.dumps({"summary": summary, "rows": rows}, indent=2),
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2))
    print(f"result: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
