#!/usr/bin/env python3
"""LongMemEval QA with a DeepSeek agent: old recall payload vs tiered recall.

Each arm answers the same questions with the same prompt and the same
`cam_mem_show` tool; only the cam binary behind `cam_recall` differs:

  baseline — `--baseline` binary (full bodies, every score field)
  slim     — candidate binary, default recall (slim JSON, full bodies)
  tiered   — candidate binary with `snippets: true` (long lower-ranked bodies clipped)

Accuracy is graded by a DeepSeek judge. Tokens are DeepSeek `usage` summed over
every answering call, tool rounds included; judge calls are counted apart.

Usage (DEEPSEEK_API_KEY / BASE_URL / MODEL in .deepseek.env or .env):
  python3 eval/recall_payload/llm_qa.py --baseline /tmp/old/cam --sample 120
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))

from eval_paths import cam_binary, dataset  # noqa: E402
from run import Mcp, ingest, load_longmemeval  # noqa: E402

MAX_TOOL_ROUNDS = 4
SYSTEM = (
    "You answer questions about the user's past chat sessions. Your memory tool "
    "already ran cam_recall for the question; its JSON result is in the user "
    "message. A hit with body_truncated holds only a snippet: call cam_mem_show "
    "with its id when you need the full session. Answer briefly with the key "
    "facts. If the memories do not contain the answer, say you don't know."
)
STRICT = (
    " A snippet is rarely enough: before answering, call cam_mem_show on every "
    "body_truncated hit unless the full bodies already answer the question completely. "
    "Later sessions may update or add to earlier ones."
)
TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "cam_mem_show",
            "description": "Read one memory in full by id.",
            "parameters": {
                "type": "object",
                "properties": {"id": {"type": "string", "description": "Memory id from cam_recall."}},
                "required": ["id"],
            },
        },
    }
]


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


def chat(messages: list[dict], max_tokens: int, tools: list | None = None) -> dict:
    key = os.environ.get("DEEPSEEK_API_KEY") or ""
    if not key:
        raise SystemExit("DEEPSEEK_API_KEY missing")
    base = (os.environ.get("DEEPSEEK_BASE_URL") or os.environ.get("BASE_URL") or "https://api.deepseek.com").rstrip("/")
    payload = {
        "model": os.environ.get("MODEL") or "deepseek-flash",
        "messages": messages,
        "temperature": 0,
        "max_tokens": max_tokens,
        "thinking": {"type": "disabled"},
    }
    if tools:
        payload["tools"] = tools
    last = ""
    for attempt in range(4):
        proc = subprocess.run(
            ["curl", "-sS", "-4", "--max-time", "180", f"{base}/chat/completions",
             "-H", f"Authorization: Bearer {key}", "-H", "Content-Type: application/json",
             "--data-binary", "@-"],
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
            last = json.dumps(data["error"])
            time.sleep(2 * (attempt + 1))
            continue
        return data
    raise RuntimeError(f"LLM failed: {last[:500]}")


def answer(
    mcp: Mcp, question: str, question_date: str, limit: int, system: str, snippets: bool
) -> dict:
    args = {"query": question, "limit": limit, "hash_embed": True}
    if snippets:
        args["snippets"] = True
    recall_text = mcp.call_text("cam_recall", args)
    messages = [
        {"role": "system", "content": system},
        {
            "role": "user",
            "content": f"Question date: {question_date}\nQuestion: {question}\n\ncam_recall result:\n{recall_text}",
        },
    ]
    prompt_tokens = completion_tokens = tool_calls = 0
    text = ""
    for round_ in range(MAX_TOOL_ROUNDS + 1):
        data = chat(messages, 300, TOOLS if round_ < MAX_TOOL_ROUNDS else None)
        usage = data.get("usage") or {}
        prompt_tokens += int(usage.get("prompt_tokens") or 0)
        completion_tokens += int(usage.get("completion_tokens") or 0)
        msg = (data.get("choices") or [{}])[0].get("message") or {}
        calls = msg.get("tool_calls") or []
        if not calls:
            text = (msg.get("content") or "").strip()
            break
        messages.append({"role": "assistant", "content": msg.get("content") or "", "tool_calls": calls})
        for call in calls:
            tool_calls += 1
            try:
                mem_id = json.loads(call["function"].get("arguments") or "{}").get("id", "")
                result = mcp.call_text("cam_mem_show", {"id": mem_id})
            except Exception as exc:  # the model may pass a bad id
                result = json.dumps({"error": str(exc)})
            messages.append({"role": "tool", "tool_call_id": call["id"], "content": result})
    return {
        "answer": text,
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "tool_calls": tool_calls,
        "recall_chars": len(recall_text),
    }


def judge(question: str, gold: str, response: str, qtype: str) -> tuple[bool, int]:
    extra = ""
    if qtype == "temporal-reasoning":
        extra = " Do not penalize off-by-one errors in counts of days, weeks, or months."
    elif qtype == "knowledge-update":
        extra = " If the response mentions old information alongside the updated answer, it is still correct when the updated answer is given."
    elif qtype == "single-session-preference":
        extra = " The rubric describes the preferred response; mark yes when the response uses the user's personal information correctly."
    prompt = (
        "I will give you a question, a correct answer, and a response from a model. "
        "Answer yes if the response contains the correct answer or is equivalent to it. "
        "Answer no if it is wrong or only contains a subset of the needed information."
        f"{extra}\n\nQuestion: {question}\nCorrect answer: {gold}\nModel response: {response}\n\n"
        "Is the model response correct? Answer yes or no only."
    )
    data = chat([{"role": "user", "content": prompt}], 5)
    text = ((data.get("choices") or [{}])[0].get("message") or {}).get("content") or ""
    return "yes" in text.lower(), int((data.get("usage") or {}).get("total_tokens") or 0)


def run_instance(
    group: dict, arms: dict[str, tuple[Path, bool]], limit: int, system: str
) -> dict | None:
    q = group["queries"][0]
    inst = group["instance"]
    with tempfile.TemporaryDirectory(prefix="cam-llmqa-") as tmp:
        seed = Path(tmp) / "seed"
        seed.mkdir()
        if not ingest(arms["tiered"][0], seed, group["memories"]):
            return None
        out = {"question_id": inst["question_id"], "type": inst["question_type"]}
        for name, (cam, snippets) in arms.items():
            root = Path(tmp) / name
            shutil.copytree(seed, root)
            with Mcp(cam, root) as mcp:
                res = answer(
                    mcp, q["question"], inst.get("question_date", ""), limit, system, snippets
                )
            ok, jt = judge(q["question"], str(inst["answer"]), res["answer"], inst["question_type"])
            res.update({"correct": ok, "judge_tokens": jt})
            out[name] = res
    marks = " ".join(f"{n}={'+' if out[n]['correct'] else '-'}{out[n]['prompt_tokens']}" for n in arms)
    print(f"  {out['question_id']:<24} {marks}", flush=True)
    return out


def main() -> int:
    load_env()
    p = argparse.ArgumentParser()
    p.add_argument("--baseline", type=Path, required=True)
    p.add_argument("--candidate", type=Path, default=None)
    p.add_argument("--sample", type=int, default=120)
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--limit", type=int, default=3)
    p.add_argument("--workers", type=int, default=4)
    p.add_argument("--strict", action="store_true", help="push the agent to open every snippet")
    args = p.parse_args()
    system = SYSTEM + (STRICT if args.strict else "")
    candidate = (args.candidate or cam_binary()).resolve()
    arms = {
        "baseline": (args.baseline.resolve(), False),
        "slim": (candidate, False),
        "tiered": (candidate, True),
    }

    groups = load_longmemeval(dataset("longmemeval"), args.sample, args.seed)

    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        rows = [r for r in pool.map(lambda g: run_instance(g, arms, args.limit, system), groups) if r]

    summary: dict = {
        "model": os.environ.get("MODEL"),
        "n": len(rows),
        "skipped_non_english": len(groups) - len(rows),
        "limit": args.limit,
        "strict": args.strict,
    }
    for name in arms:
        rs = [r[name] for r in rows]
        summary[name] = {
            "accuracy": round(statistics.mean(r["correct"] for r in rs), 4),
            "prompt_tokens": sum(r["prompt_tokens"] for r in rs),
            "completion_tokens": sum(r["completion_tokens"] for r in rs),
            "mean_prompt_tokens": round(statistics.mean(r["prompt_tokens"] for r in rs), 1),
            "tool_calls": sum(r["tool_calls"] for r in rs),
            "queries_with_tool_calls": sum(1 for r in rs if r["tool_calls"]),
        }
    base = summary["baseline"]["prompt_tokens"]
    for name in arms:
        if name == "baseline":
            continue
        summary[name]["prompt_token_change"] = (
            round(summary[name]["prompt_tokens"] / base - 1, 4) if base else 0.0
        )
        summary[name]["only_baseline_correct"] = [
            r["question_id"] for r in rows if r["baseline"]["correct"] and not r[name]["correct"]
        ]
        summary[name]["only_this_correct"] = [
            r["question_id"] for r in rows if r[name]["correct"] and not r["baseline"]["correct"]
        ]
    by_type: dict = {}
    for r in rows:
        d = by_type.setdefault(r["type"], {"n": 0, **{name: 0 for name in arms}})
        d["n"] += 1
        for name in arms:
            d[name] += r[name]["correct"]
    summary["by_type"] = by_type

    results = HERE / "results"
    results.mkdir(exist_ok=True)
    dest = results / f"llm-qa-k{args.limit}-{time.strftime('%Y%m%d-%H%M%S')}.json"
    dest.write_text(json.dumps({"summary": summary, "rows": rows}, indent=2, ensure_ascii=False))
    print(json.dumps(summary, indent=2))
    print(f"wrote {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
