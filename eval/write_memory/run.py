#!/usr/bin/env python3
"""Does a model store one fact per memory when it writes cam_add itself?

Two arms see the same notes and the same cam_add tool. `legacy` is told to
store one full write-up. `split` is told the current rule: one fact or one
problem's solution path per memory, with a memories array for the rest.

The model calls cam_add on a throwaway project. Scoring is lexical against
cases.json: a fact is exclusive when one memory holds all of its phrases and
no other fact. Links count when those memories share an edge, or the facts
land in the same memory.

Usage (DEEPSEEK_API_KEY in .deepseek.env or .env):
  python3 eval/write_memory/test_score.py
  python3 eval/write_memory/run.py
  python3 eval/write_memory/run.py --arm split --case tool-truncation
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))

from cases import ensure_cases  # noqa: E402
from eval_paths import cam_binary, embed_env  # noqa: E402
from score import mean_rate, score_case  # noqa: E402

ARMS = ("legacy", "split")
MAX_ROUNDS = 3
SPLIT_RULE = (
    "Store what you just learned with cam_add. One memory is one fact, or the "
    "solution path for one problem (symptom, cause, change, and how to verify). "
    "If two pieces can be asked about separately, or can become outdated separately, "
    "store them as two memories. A body holds only the detail that one memory needs. "
    "Put several memories from the same solution in one cam_add call as memories. "
    "Inside that array, parent, supersedes, and links may use #0, #1, ... for other items."
)
LEGACY_RULE = (
    "Store what you just learned as one cam memory. Call cam_add once. "
    "summary is a one-line English title. body is the full write-up of the investigation."
)
ADD_PROPERTIES = {
    "summary": {"type": "string", "description": "One-line English title for this one memory."},
    "body": {"type": "string", "description": "Detail this one memory needs. Supports non-Latin characters."},
    "parent": {"type": "string"},
    "supersedes": {"type": "string"},
    "links": {"type": "array", "items": {"type": "string"}},
    "memories": {
        "type": "array",
        "description": "Several memories. parent, supersedes, and links may use #0, #1, ...",
        "items": {
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "body": {"type": "string"},
                "parent": {"type": "string"},
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
    def __init__(self, project: Path):
        env = embed_env(hash_embed=True)
        self.proc = subprocess.Popen(
            [str(cam_binary()), "mcp", "--project", str(project)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
        self.next_id = 1
        self.request(
            "initialize",
            {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "write-memory", "version": "0"},
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


def tool_schema(arm: str) -> list[dict]:
    properties = dict(ADD_PROPERTIES)
    if arm == "legacy":
        properties.pop("memories")
        required = ["summary", "body"]
    else:
        required = []
    return [
        {
            "type": "function",
            "function": {
                "name": "cam_add",
                "description": "Store memories in the project.",
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                },
            },
        }
    ]


def system_prompt(arm: str) -> str:
    rule = SPLIT_RULE if arm == "split" else LEGACY_RULE
    return (
        f"{rule} Summary must be English. Call the cam_add tool. "
        "Do not answer in prose until the tool call is done."
    )


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
        if shown.get("latest", True) and "body" in shown:
            memories.append(shown)
    return memories


def write_case(arm: str, case: dict) -> dict:
    project = Path(tempfile.mkdtemp(prefix=f"cam-write-{case['id']}-"))
    mcp = Mcp(project)
    calls = []
    prompt_tokens = completion_tokens = 0
    try:
        messages = [
            {"role": "system", "content": system_prompt(arm)},
            {"role": "user", "content": case["note"]},
        ]
        tools = tool_schema(arm)
        final = ""
        for _ in range(MAX_ROUNDS):
            data = chat(messages, tools)
            usage = data.get("usage") or {}
            prompt_tokens += int(usage.get("prompt_tokens") or 0)
            completion_tokens += int(usage.get("completion_tokens") or 0)
            message = (data.get("choices") or [{}])[0].get("message") or {}
            tool_calls = message.get("tool_calls") or []
            if not tool_calls:
                final = (message.get("content") or "").strip()
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
                arguments["hash_embed"] = True
                result = mcp.call_tool("cam_add", arguments)
                calls.append({"arguments": arguments, "result": result})
                messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": call.get("id"),
                        "content": json.dumps(result),
                    }
                )
        memories = stored_memories(mcp)
    finally:
        mcp.close()
    scored = score_case(memories, case)
    return {
        "case": case["id"],
        "expect": case["expect"],
        "arm": arm,
        "calls": len(calls),
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
            "final": final,
        "stored": [
            {
                "id": memory["id"],
                "summary": memory.get("summary"),
                "body_chars": len(memory.get("body") or ""),
                "links": memory.get("links") or [],
            }
            for memory in memories
        ],
        "score": scored,
    }


def print_table(rows: list[dict]) -> None:
    print(
        f"{'arm':<8} {'case':<24} {'n':>3} {'body':>6} {'cover':>6} {'excl':>6} {'links':>7}"
    )
    for row in rows:
        score = row["score"]
        links = f"{score['link_hits']}/{score['link_total']}"
        print(
            f"{row['arm']:<8} {row['case']:<24} {score['memories']:>3} "
            f"{score['max_body_chars']:>6} {score['coverage']:>6.2f} "
            f"{score['exclusive_rate']:>6.2f} {links:>7}"
        )
    for arm in ARMS:
        arm_rows = [row["score"] for row in rows if row["arm"] == arm]
        if not arm_rows:
            continue
        print(
            f"{arm}: memories {mean_rate(arm_rows, 'memories'):.1f}, "
            f"max body {mean_rate(arm_rows, 'max_body_chars'):.0f} chars, "
            f"coverage {mean_rate(arm_rows, 'coverage'):.2f}, "
            f"exclusive {mean_rate(arm_rows, 'exclusive_rate'):.2f}"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arm", choices=ARMS, action="append")
    parser.add_argument("--case", action="append")
    args = parser.parse_args()
    load_env()
    cases = json.loads(ensure_cases().read_text())
    if args.case:
        cases = [case for case in cases if case["id"] in args.case]
    arms = args.arm or list(ARMS)
    rows = []
    for case in cases:
        for arm in arms:
            print(f"writing {case['id']} / {arm}", flush=True)
            row = write_case(arm, case)
            rows.append(row)
            score = row["score"]
            print(
                f"  memories={score['memories']} max_body={score['max_body_chars']} "
                f"exclusive={score['exclusive']}/{score['facts']}",
                flush=True,
            )
    print_table(rows)
    out_dir = HERE / "results"
    out_dir.mkdir(exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = out_dir / f"write-memory-{stamp}.json"
    path.write_text(
        json.dumps(
            {
                "model": os.environ.get("MODEL") or "deepseek-flash",
                "rows": rows,
            },
            indent=2,
        )
    )
    print(f"wrote {path}")


if __name__ == "__main__":
    main()
