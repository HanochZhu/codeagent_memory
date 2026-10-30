#!/usr/bin/env python3
"""Compare cam and Mem0 on HaluMem with one DeepSeek judge.

Every frame sees the same sessions. Extraction uses that frame's own writer,
with DeepSeek as the language model. Scoring uses the HaluMem integrity,
accuracy, update, and question rubrics. The question reader prompt is shared.

MemOS, Memobase, and Zep are not started. MemOS needs Neo4j and Qdrant.
Memobase needs its server. Zep Cloud has no API key, and Graphiti's Kuzu
driver fails before it can store or search an episode.

Usage:
  python3 eval/halumem/test_judge.py
  /tmp/halu-venv/bin/python eval/halumem/compare.py --users 2
  /tmp/halu-venv/bin/python eval/halumem/compare.py --frames cam --users 1 --sessions 1
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import traceback
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))

from eval_paths import add_hash_embed_argument  # noqa: E402
from judge import JudgeCache, combine, judge_user  # noqa: E402
from llm import load_env, model_name  # noqa: E402
from run import ensure_data, load_users  # noqa: E402
from systems import QA_K, SYSTEMS, UPDATE_K, answer  # noqa: E402


def extract_user(system, user: dict, session_limit: int | None) -> dict:
    sessions = user["sessions"][: session_limit or None]
    stored = []
    for index, session in enumerate(sessions, 1):
        new_session = {
            "memory_points": session.get("memory_points") or [],
            "dialogue": session.get("dialogue") or [],
            "questions": [],
            "is_generated_qa_session": bool(session.get("is_generated_qa_session")),
        }
        try:
            extracted = system.add_session(session)
        except Exception as exc:
            extracted = []
            new_session["add_error"] = str(exc)[:400]
            print(f"  session {index}/{len(sessions)} add failed: {exc}", flush=True)
        new_session["extracted_memories"] = extracted
        if not new_session["is_generated_qa_session"]:
            for point in new_session["memory_points"]:
                originals = [item for item in (point.get("original_memories") or []) if item]
                if str(point.get("is_update")) != "True" or not originals:
                    continue
                try:
                    point["memories_from_system"] = system.search(point["memory_content"], UPDATE_K)
                except Exception as exc:
                    point["memories_from_system"] = []
                    point["search_error"] = str(exc)[:300]
        for question in session.get("questions") or []:
            copied = dict(question)
            try:
                memories = system.search(question.get("question") or "", QA_K)
                copied["system_response"] = answer("\n".join(memories), question.get("question") or "")
            except Exception as exc:
                copied["system_response"] = ""
                copied["answer_error"] = str(exc)[:300]
            new_session["questions"].append(copied)
        stored.append(new_session)
        print(
            f"  session {index}/{len(sessions)} extracted={len(extracted)}",
            flush=True,
        )
    return {
        "uuid": user["uuid"],
        "sessions": stored,
    }


def run_frame(name: str, users: list[dict], session_limit: int | None, workers: int, out_dir: Path, hash_embed: bool) -> dict:
    frame_dir = out_dir / name
    frame_dir.mkdir(parents=True, exist_ok=True)
    cache = JudgeCache(frame_dir / "judge-cache.sqlite")
    system_cls = SYSTEMS[name]
    rows = []
    record_rows = []
    errors = []
    for index, user in enumerate(users, 1):
        print(f"{name} user {index}/{len(users)} {user['uuid']}", flush=True)
        artifact_path = frame_dir / f"{user['uuid']}.json"
        system = None
        try:
            system = system_cls(hash_embed)
            artifact = extract_user(system, user, session_limit)
            artifact["frame"] = name
            artifact["note"] = system_cls.note
            artifact_path.write_text(json.dumps(artifact))
        except Exception as exc:
            errors.append({"uuid": user["uuid"], "error": str(exc)[:500], "trace": traceback.format_exc()[-1500:]})
            print(f"  user failed: {exc}", flush=True)
            continue
        finally:
            if system is not None:
                system.close()
        print(f"  judging {user['uuid']}", flush=True)
        judged = judge_user(artifact, cache, workers)
        judged["uuid"] = user["uuid"]
        judged["sessions"] = len(artifact["sessions"])
        rows.append({"uuid": user["uuid"], "sessions": len(artifact["sessions"]), "score": judged["score"]})
        record_rows.append(judged["records"])
        (frame_dir / "partial.json").write_text(
            json.dumps({"rows": rows, "score": combine(record_rows)}, indent=2)
        )
        print(
            "  "
            + " ".join(
                f"{key}={judged['score'][key]}"
                for key in ("recall", "f1", "resistance", "update_correct", "qa_correct")
            ),
            flush=True,
        )
    overall = combine(record_rows) if record_rows else {}
    return {
        "frame": name,
        "note": system_cls.note,
        "errors": errors,
        "score": overall,
        "rows": rows,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frames", default="cam,mem0")
    parser.add_argument("--users", type=int, default=2)
    parser.add_argument("--sessions", type=int, default=0, help="sessions per user; 0 means all")
    parser.add_argument("--judge-workers", type=int, default=6)
    add_hash_embed_argument(parser)
    args = parser.parse_args()
    load_env()
    names = [name.strip() for name in args.frames.split(",") if name.strip()]
    unknown = [name for name in names if name not in SYSTEMS]
    if unknown:
        raise SystemExit(f"unknown frame: {', '.join(unknown)}")
    users = load_users(ensure_data())[: args.users]
    limit = args.sessions or None
    out_dir = HERE / "results" / (
        f"compare-{'-'.join(names)}-{os.getpid()}-{datetime.now().strftime('%Y%m%d-%H%M%S')}"
    )
    out_dir.mkdir(parents=True)
    summary = {
        "dataset": "HaluMem-Medium",
        "users": len(users),
        "user_ids": [user["uuid"] for user in users],
        "sessions_per_user": args.sessions or "all",
        "model": model_name(),
        "judge": "HaluMem integrity, accuracy, update, and question prompts",
        "reader": "one shared DeepSeek prompt for every frame",
        "not_run": {
            "memos": "MemOS needs Neo4j and Qdrant. Docker is not available and no MemOS key is set.",
            "memobase": "Memobase needs its server and database. No project token is set.",
            "zep": (
                "No Zep API key. Graphiti 0.31 on Kuzu raises before add "
                "because KuzuDriver has no _database, and search requires a "
                "full-text index the driver never creates. Neo4j is not installed."
            ),
        },
        "frames": {},
    }
    path = out_dir / "summary.json"
    for name in names:
        summary["frames"][name] = run_frame(name, users, limit, args.judge_workers, out_dir, args.hash_embed)
        path.write_text(json.dumps(summary, indent=2))
    print(json.dumps({name: frame.get("score") for name, frame in summary["frames"].items()}, indent=2))
    print(f"wrote {path}")


if __name__ == "__main__":
    main()
