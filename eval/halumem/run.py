#!/usr/bin/env python3
"""Does cam's one-fact write rule cover HaluMem points and link updates?

Sessions of one user are shown in order. The model calls cam_add. Scoring is
lexical against memory_points: see score.py. The default is the first two
users. --users 20 runs HaluMem-Medium.

Usage (DEEPSEEK_API_KEY in .deepseek.env or .env):
  python3 eval/halumem/test_score.py
  python3 eval/halumem/run.py
  python3 eval/halumem/run.py --users 1 --sessions 3
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))

from eval_paths import POTION_MODEL, add_hash_embed_argument, cam_binary, dataset, embed_env  # noqa: E402
from score import score_bank  # noqa: E402

DATA_URL = (
    "https://huggingface.co/datasets/IAAR-Shanghai/HaluMem/resolve/main/HaluMem-Medium.jsonl"
)
RULE = (
    "Store what this dialogue states about the user with cam_add. One memory is one fact. "
    "If two pieces can be asked about separately, store them as two memories. "
    "Put the memories from this dialogue in one cam_add call as memories. "
    "If a new fact replaces one of the existing memories listed below, set supersedes to that id. "
    "Do not store a claim the user did not make. Summary must be English. "
    "Call cam_add. Do not answer in prose until the tool call is done."
)
ADD_PROPERTIES = {
    "summary": {"type": "string"},
    "body": {"type": "string"},
    "supersedes": {"type": "string"},
    "links": {"type": "array", "items": {"type": "string"}},
    "memories": {
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "body": {"type": "string"},
                "supersedes": {"type": "string"},
                "links": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["summary", "body"],
        },
    },
}


def load_env() -> None:
    for name in (".deepseek.env", ".env"):
        path = REPO / name
        if not path.exists():
            continue
        for raw in path.read_text().splitlines():
            line = raw.strip()
            if line and not line.startswith("#") and "=" in line:
                key, value = line.split("=", 1)
                os.environ.setdefault(key.strip(), value.strip().strip("'\""))


def ensure_data() -> Path:
    path = dataset("halumem")
    if path.exists() and path.stat().st_size > 1_000_000:
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_suffix(path.suffix + ".partial")
    request = urllib.request.Request(DATA_URL, headers={"User-Agent": "cam-halumem"})
    with urllib.request.urlopen(request, timeout=180) as response, partial.open("wb") as out:
        while chunk := response.read(1024 * 1024):
            out.write(chunk)
    partial.replace(path)
    return path


def load_users(path: Path) -> list[dict]:
    users = []
    for line in path.read_text().splitlines():
        if line.strip():
            users.append(json.loads(line))
    return users


def chat(messages: list[dict], tools: list[dict]) -> dict:
    key = os.environ.get("DEEPSEEK_API_KEY") or ""
    if not key:
        raise SystemExit("DEEPSEEK_API_KEY missing")
    base = (
        os.environ.get("DEEPSEEK_BASE_URL") or os.environ.get("BASE_URL") or "https://api.deepseek.com"
    ).rstrip("/")
    payload = {
        "model": os.environ.get("MODEL") or "deepseek-flash",
        "messages": messages,
        "temperature": 0,
        "max_tokens": 4000,
        "thinking": {"type": "disabled"},
        "tools": tools,
    }
    last = ""
    for attempt in range(4):
        proc = subprocess.run(
            [
                "curl", "-sS", "-4", "--max-time", "180", f"{base}/chat/completions",
                "-H", f"Authorization: Bearer {key}", "-H", "Content-Type: application/json",
                "--data-binary", "@-",
            ],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            check=False,
        )
        last = proc.stdout or proc.stderr
        try:
            data = json.loads(proc.stdout)
        except json.JSONDecodeError:
            time.sleep(2 * (attempt + 1))
            continue
        if data.get("error"):
            last = json.dumps(data["error"])[:500]
            time.sleep(2 * (attempt + 1))
            continue
        return data
    raise RuntimeError(f"LLM failed: {last[:500]}")


class Mcp:
    def __init__(self, project: Path, hash_embed: bool):
        self.proc = subprocess.Popen(
            [str(cam_binary()), "mcp", "--project", str(project)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=embed_env(hash_embed=hash_embed),
        )
        self.next_id = 1
        self.request(
            "initialize",
            {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "halumem", "version": "0"},
            },
        )
        self.notify("notifications/initialized")

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.kill()
        self.proc.wait(timeout=5)

    def request(self, method: str, params: dict) -> dict:
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.next_id += 1
        return self._roundtrip(message)

    def notify(self, method: str) -> None:
        self._roundtrip({"jsonrpc": "2.0", "method": method})

    def _roundtrip(self, message: dict) -> dict:
        assert self.proc.stdin and self.proc.stdout
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()
        if "id" not in message:
            return {}
        line = self.proc.stdout.readline()
        if not line:
            err = self.proc.stderr.read() if self.proc.stderr else ""
            raise RuntimeError(f"cam mcp closed: {err[:400]}")
        return json.loads(line)

    def call_tool(self, name: str, arguments: dict) -> dict:
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        result = response.get("result") or {}
        text = ((result.get("content") or [{}])[0].get("text")) or ""
        if response.get("error"):
            return {"error": response["error"], "is_error": True}
        try:
            payload = json.loads(text) if text else {}
        except json.JSONDecodeError:
            payload = {"raw": text}
        if result.get("isError"):
            payload = payload if isinstance(payload, dict) else {"raw": payload}
            payload["is_error"] = True
        return payload


def tool_schema() -> list[dict]:
    return [
        {
            "type": "function",
            "function": {
                "name": "cam_add",
                "description": "Store one fact, or several facts from this dialogue.",
                "parameters": {"type": "object", "properties": ADD_PROPERTIES},
            },
        }
    ]


def walk_ids(nodes: list[dict]) -> list[str]:
    found = []
    for node in nodes:
        found.append(node["id"])
        found.extend(walk_ids(node.get("children") or []))
    return found


def stored_memories(mcp: Mcp) -> list[dict]:
    tree = mcp.call_tool("cam_mem_tree", {})
    nodes = tree if isinstance(tree, list) else []
    memories = []
    for mem_id in walk_ids(nodes):
        shown = mcp.call_tool("cam_mem_show", {"id": mem_id})
        if "body" in shown:
            memories.append(shown)
    return memories


def dialogue_text(session: dict) -> str:
    lines = []
    for turn in session.get("dialogue") or []:
        lines.append(f"{turn.get('role', 'unknown')}: {turn.get('content', '')}")
    return "\n".join(lines)


def memory_list(memories: list[dict]) -> str:
    recent = memories[-40:]
    if not recent:
        return "Existing memories: none."
    lines = [f"{item['id']}: {item.get('summary')}" for item in recent]
    return "Existing memories:\n" + "\n".join(lines)


def note_added(result: dict, known: list[dict]) -> None:
    if not isinstance(result, dict) or result.get("is_error"):
        return
    items = result.get("memories")
    if items is None and result.get("id"):
        items = [result]
    for item in items or []:
        if item.get("id"):
            known.append({"id": item["id"], "summary": item.get("summary") or item["id"]})


def write_session(mcp: Mcp, session: dict, memories: list[dict], hash_embed: bool) -> int:
    messages = [
        {"role": "system", "content": RULE},
        {"role": "user", "content": memory_list(memories) + "\n\nDialogue:\n" + dialogue_text(session)},
    ]
    tools = tool_schema()
    calls = 0
    for _ in range(3):
        data = chat(messages, tools)
        message = (data.get("choices") or [{}])[0].get("message") or {}
        tool_calls = message.get("tool_calls") or []
        if not tool_calls:
            break
        messages.append(
            {"role": "assistant", "content": message.get("content") or "", "tool_calls": tool_calls}
        )
        for call in tool_calls:
            raw = call.get("function", {}).get("arguments") or "{}"
            try:
                arguments = json.loads(raw)
            except json.JSONDecodeError:
                arguments = {}
            arguments["hash_embed"] = hash_embed
            result = mcp.call_tool("cam_add", arguments)
            note_added(result, memories)
            calls += 1
            messages.append(
                {"role": "tool", "tool_call_id": call.get("id"), "content": json.dumps(result)}
            )
    return calls


def points_of(user: dict, session_limit: int | None) -> list[dict]:
    sessions = user["sessions"][: session_limit or None]
    points = []
    for session in sessions:
        points.extend(session.get("memory_points") or [])
    return points


def run_user(user: dict, session_limit: int | None, hash_embed: bool) -> dict:
    project = Path(tempfile.mkdtemp(prefix="cam-halu-"))
    mcp = Mcp(project, hash_embed)
    sessions = user["sessions"][: session_limit or None]
    known: list[dict] = []
    errors: list[dict] = []
    try:
        for index, session in enumerate(sessions, 1):
            try:
                write_session(mcp, session, known, hash_embed)
            except Exception as exc:
                errors.append({"session": index, "error": str(exc)[:400]})
                print(f"  session {index}/{len(sessions)} failed: {exc}", flush=True)
                continue
            print(f"  session {index}/{len(sessions)} stored={len(known)}", flush=True)
        memories = stored_memories(mcp)
    finally:
        mcp.close()
    scored = score_bank(memories, points_of(user, session_limit))
    return {"uuid": user["uuid"], "sessions": len(sessions), "errors": errors, "score": scored}


def mean(rows: list[dict], key: str) -> float | None:
    values = [row["score"][key] for row in rows if row["score"][key] is not None]
    if not values:
        return None
    return sum(values) / len(values)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--users", type=int, default=2)
    parser.add_argument("--sessions", type=int, default=0, help="sessions per user; 0 means all")
    add_hash_embed_argument(parser)
    args = parser.parse_args()
    load_env()
    users = load_users(ensure_data())[: args.users]
    limit = args.sessions or None
    rows = []
    out_dir = HERE / "results"
    out_dir.mkdir(exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = out_dir / f"halumem-medium-{stamp}.json"

    def dump() -> None:
        summary = {
            "dataset": "HaluMem-Medium",
            "users": len(rows),
            "sessions_per_user": args.sessions or "all",
            "model": os.environ.get("MODEL") or "deepseek-flash",
            "embedder": "hash" if args.hash_embed else POTION_MODEL,
            "matcher": "content-word coverage >= 0.55",
            "recall": mean(rows, "recall"),
            "precision": mean(rows, "precision"),
            "f1": mean(rows, "f1"),
            "update_rate": mean(rows, "update_rate"),
            "resistance": mean(rows, "resistance"),
            "rows": rows,
        }
        path.write_text(json.dumps(summary, indent=2))

    for index, user in enumerate(users, 1):
        print(f"user {index}/{len(users)} {user['uuid']}", flush=True)
        rows.append(run_user(user, limit, args.hash_embed))
        score = rows[-1]["score"]
        dump()
        print(
            f"  recall={score['recall']:.2f} precision={score['precision']:.2f} "
            f"f1={score['f1']:.2f} update={score['update_hits']}/{score['update_total']} "
            f"resist={score['resistance']}",
            flush=True,
        )
    print(json.dumps({key: json.loads(path.read_text())[key] for key in (
        "users", "recall", "precision", "f1", "update_rate", "resistance"
    )}, indent=2))
    print(f"wrote {path}")


if __name__ == "__main__":
    main()
