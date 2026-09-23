#!/usr/bin/env python3
"""Multi-turn DeepSeek QA: cam retrieval vs full dump, accuracy + tokens.

Two tracks:

  solutions — coding-agent-life-v1 (15 sessions, 15 queries, multi-session)
  code      — this repo's src/ after `cam index`, plus DESIGN.md as a solution

Each track runs a single conversation twice:

  full — dump every session / every .rs file into the system prompt
  cam  — each turn injects `cam recall` (+ surgical `read`/`ref` on code)

Token counts come from DeepSeek `usage` on answering calls only (judge
calls are recorded separately). Grade with a DeepSeek JSON judge.

Usage:
  python3 eval/llm_multiturn/run.py
  python3 eval/llm_multiturn/run.py --track solutions
  python3 eval/llm_multiturn/run.py --track code
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from eval_paths import dataset  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
DEFAULT_LIFE = dataset("coding_life")
IDENT_RE = re.compile(r"\b[A-Za-z][A-Za-z0-9_]{2,}\b")
STOP = {
    "the",
    "and",
    "for",
    "how",
    "what",
    "which",
    "who",
    "does",
    "did",
    "was",
    "were",
    "when",
    "where",
    "with",
    "from",
    "into",
    "that",
    "this",
    "function",
    "true",
    "false",
    "cam",
    "src",
}

CODE_QUESTIONS = [
    {
        "id": "c-001",
        "type": "symbol-read",
        "question": "How does fuse_scores combine BM25 and vector scores, and what does it return?",
        "answer": "Min-max normalize each path to [0,1] then sum; returns Vec<(id, fused score)> sorted descending.",
    },
    {
        "id": "c-002",
        "type": "callers",
        "question": "Which production function calls fuse_scores?",
        "answer": "recall() in src/memory/recall.rs (tests in the same file also call it).",
    },
    {
        "id": "c-003",
        "type": "symbol-read",
        "question": "What does tokenize_for_fts do for Chinese text before BM25?",
        "answer": "jieba.cut the summary+body, then join tokens with spaces so FTS5 can match CJK.",
    },
    {
        "id": "c-004",
        "type": "symbol-read",
        "question": "What is the Ebbinghaus retention formula, initial S, and when is needs_update true?",
        "answer": "R = exp(-t / S); initial S = 7 days; needs_update when R < 0.3; successful recall refreshes C0 and S *= 1.7.",
    },
    {
        "id": "c-005",
        "type": "followup",
        "question": "If a solution memory is stale, does cam delete it? How do you update it from the CLI?",
        "answer": "Never deleted. cam add a new node (optionally --parent); newest on the path is latest.",
    },
    {
        "id": "c-006",
        "type": "followup",
        "question": "What is the virtual path src/memory/recall.rs/fuse_scores compared to src/memory/recall.rs?",
        "answer": "src/memory/recall.rs is the file; src/memory/recall.rs/fuse_scores is the symbol inside that file.",
    },
]


def load_dotenv(path: Path) -> None:
    if not path.exists():
        return
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ.setdefault(key.strip(), value.strip().strip('"').strip("'"))


def cam_bin() -> Path:
    env = os.environ.get("CAM_BIN")
    if env:
        return Path(env)
    debug = REPO / "target" / "debug" / "cam"
    if debug.exists():
        return debug
    subprocess.check_call(["cargo", "build", "-q"], cwd=REPO)
    return debug


def cam_run(
    cam: Path,
    root: Path,
    args: list[str],
    stdin: str | None = None,
    json_out: bool = False,
) -> str:
    cmd = [str(cam), "--path", str(root), *args]
    if json_out:
        cmd.insert(1, "--json")
    proc = subprocess.run(
        cmd,
        cwd=root,
        input=stdin,
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}\n{proc.stderr}")
    return proc.stdout


def cam_json(cam: Path, root: Path, args: list[str], stdin: str | None = None):
    return json.loads(cam_run(cam, root, args, stdin=stdin, json_out=True) or "null")


def llm_config() -> tuple[str, str, str]:
    key = os.environ.get("DEEPSEEK_API_KEY") or ""
    if not key:
        raise SystemExit("DEEPSEEK_API_KEY missing in .env")
    base = (
        os.environ.get("DEEPSEEK_BASE_URL")
        or os.environ.get("BASE_URL")
        or "https://api.deepseek.com"
    ).rstrip("/")
    model = os.environ.get("MODEL") or "deepseek-flash"
    return key, base, model


def chat(
    key: str,
    base: str,
    model: str,
    messages: list[dict],
    max_tokens: int = 400,
) -> dict:
    url = f"{base}/chat/completions"
    payload = {
        "model": model,
        "messages": messages,
        "temperature": 0,
        "max_tokens": max_tokens,
        # Flash thinks by default; CoT would eat max_tokens and leave content empty.
        "thinking": {"type": "disabled"},
    }
    proc = subprocess.run(
        [
            "curl",
            "-sS",
            "-4",
            "--max-time",
            "120",
            url,
            "-H",
            f"Authorization: Bearer {key}",
            "-H",
            "Content-Type: application/json",
            "-d",
            json.dumps(payload),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"curl failed: {proc.stderr or proc.stdout}"[:800])
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"LLM non-JSON: {proc.stdout[:800]}") from exc
    if data.get("error"):
        raise RuntimeError(f"LLM error: {data['error']}")
    choice = (data.get("choices") or [{}])[0].get("message") or {}
    text = (choice.get("content") or choice.get("reasoning_content") or "").strip()
    usage = data.get("usage") or {}
    details = usage.get("completion_tokens_details") or {}
    return {
        "text": text,
        "prompt_tokens": int(usage.get("prompt_tokens") or 0),
        "completion_tokens": int(usage.get("completion_tokens") or 0),
        "reasoning_tokens": int(details.get("reasoning_tokens") or 0),
        "total_tokens": int(usage.get("total_tokens") or 0),
        "raw_model": data.get("model") or model,
        "finish_reason": (data.get("choices") or [{}])[0].get("finish_reason"),
    }


def format_hits(hits: list[dict], body_limit: int = 600) -> str:
    if not hits:
        return "(no memories)"
    parts = []
    for h in hits:
        body = (h.get("body") or "")[:body_limit]
        parts.append(f"- {h.get('summary', '')}\n{body}")
    return "\n\n".join(parts)


def idents_in(text: str) -> list[str]:
    seen: list[str] = []
    for tok in IDENT_RE.findall(text):
        if tok.lower() in STOP or tok in seen:
            continue
        seen.append(tok)
    return seen[:8]


def solutions_cam_context(cam: Path, root: Path, question: str, hash_embed: bool) -> str:
    extra = ["--hash-embed"] if hash_embed else []
    hits = cam_json(cam, root, ["recall", question, "--limit", "5", *extra])
    return format_hits(hits if isinstance(hits, list) else [])


def _try_cam(cam: Path, root: Path, args: list[str]) -> str:
    try:
        return cam_run(cam, root, args).strip()
    except Exception:
        return ""


def code_cam_context(cam: Path, root: Path, question: str, hash_embed: bool) -> str:
    chunks: list[str] = []
    extra = ["--hash-embed"] if hash_embed else []
    try:
        hits = cam_json(cam, root, ["recall", question, "--limit", "3", *extra])
        chunks.append("## recall\n" + format_hits(hits if isinstance(hits, list) else [], 500))
    except Exception as exc:
        chunks.append(f"## recall\n(error: {exc})")
    rust_files = [str(p.relative_to(root)) for p in sorted((root / "src").rglob("*.rs"))]
    for ident in idents_in(question):
        for dir_ in ("in", "out"):
            text = _try_cam(cam, root, ["ref", ident, "--dir", dir_])
            if text:
                chunks.append(f"## ref {ident} --dir {dir_}\n{text[:1200]}")
        for rel in rust_files:
            body = _try_cam(cam, root, ["read", f"{rel}/{ident}"])
            if body and "specify a file" not in body.lower():
                chunks.append(f"## read {rel}/{ident}\n{body[:2000]}")
                break
    if not chunks:
        return "(empty cam context)"
    joined = "\n\n".join(chunks)
    return joined[:8000]


def dump_sessions(sessions: list[dict]) -> str:
    blocks = []
    for s in sessions:
        blocks.append(f"### {s['id']}  {s.get('timestamp') or ''}\n{s['content']}")
    return "\n\n".join(blocks)


def dump_rust(root: Path) -> str:
    parts = []
    for path in sorted(root.rglob("*.rs")):
        rel = path.relative_to(root)
        parts.append(f"// ===== {rel} =====\n{path.read_text()}")
    design = root / "DESIGN.md"
    if design.exists():
        parts.append(f"# DESIGN.md\n{design.read_text()}")
    return "\n\n".join(parts)


def judge(key: str, base: str, model: str, gold: str, answer: str) -> dict:
    prompt = (
        "Grade whether the answer contains the gold answer's key facts. "
        "Paraphrase is OK. Reply JSON only.\n"
        f"Gold: {gold}\nAnswer: {answer}\n"
        'Schema: {"correct": true or false}'
    )
    out = chat(
        key,
        base,
        model,
        [{"role": "user", "content": prompt}],
        max_tokens=80,
    )
    parsed = {"correct": False}
    text = out["text"]
    try:
        start = text.find("{")
        end = text.rfind("}")
        if start >= 0 and end > start:
            parsed = json.loads(text[start : end + 1])
    except json.JSONDecodeError:
        parsed = {"correct": "true" in text.lower()}
    return {
        "correct": bool(parsed.get("correct")),
        "judge_tokens": out["total_tokens"],
        "judge_raw": text[:300],
    }


def run_conversation(
    *,
    key: str,
    base: str,
    model: str,
    questions: list[dict],
    mode: str,
    system: str,
    retrieve,
) -> dict:
    messages = [{"role": "system", "content": system}]
    rows = []
    prompt_tokens = 0
    completion_tokens = 0
    judge_tokens = 0
    t0 = time.perf_counter()
    for q in questions:
        if mode == "cam":
            ctx = retrieve(q["question"])
            user = (
                "Retrieved context (use this; do not invent missing facts):\n"
                f"{ctx}\n\nQuestion: {q['question']}\n"
                "Answer with the key facts only."
            )
        else:
            user = q["question"] + "\nAnswer with the key facts only."
        messages.append({"role": "user", "content": user})
        # keep a bounded history so cam vs full is about context size, not unbounded chat
        if len(messages) > 13:
            messages = [messages[0], *messages[-12:]]
        turn = chat(key, base, model, messages, max_tokens=350)
        # Do not keep retrieved blobs in later turns; only the question + answer.
        messages[-1] = {"role": "user", "content": q["question"]}
        messages.append({"role": "assistant", "content": turn["text"]})
        graded = judge(key, base, model, q["answer"], turn["text"])
        prompt_tokens += turn["prompt_tokens"]
        completion_tokens += turn["completion_tokens"]
        judge_tokens += graded["judge_tokens"]
        rows.append(
            {
                "id": q["id"],
                "type": q.get("type"),
                "correct": graded["correct"],
                "prompt_tokens": turn["prompt_tokens"],
                "completion_tokens": turn["completion_tokens"],
                "answer": turn["text"][:800],
            }
        )
        mark = "+" if graded["correct"] else "-"
        print(
            f"  {mark} {q['id']}  ptok={turn['prompt_tokens']}  "
            f"ctok={turn['completion_tokens']}",
            flush=True,
        )
    n = len(rows)
    return {
        "mode": mode,
        "n": n,
        "accuracy": sum(1 for r in rows if r["correct"]) / n if n else 0.0,
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": prompt_tokens + completion_tokens,
        "avg_prompt_tokens": prompt_tokens / n if n else 0.0,
        "judge_tokens": judge_tokens,
        "elapsed_s": time.perf_counter() - t0,
        "rows": rows,
    }


def setup_solutions(cam: Path, data: Path, hash_embed: bool) -> tuple[Path, list[dict], str]:
    sessions = json.loads((data / "sessions.json").read_text())
    queries = json.loads((data / "queries.json").read_text())
    tmp = Path(tempfile.mkdtemp(prefix="cam-llm-life-"))
    extra = ["--hash-embed"] if hash_embed else []
    for sess in sessions:
        body = f"{sess['id']}\n{sess.get('timestamp') or ''}\n{sess['content']}"
        summary = f"{sess['id']}: {sess['content'].splitlines()[0][:80]}"
        cam_json(cam, tmp, ["add", "--summary", summary, *extra], stdin=body)
    dump = dump_sessions(sessions)
    return tmp, queries, dump


def setup_code(cam: Path, hash_embed: bool) -> tuple[Path, list[dict], str]:
    tmp = Path(tempfile.mkdtemp(prefix="cam-llm-code-"))
    shutil.copytree(REPO / "src", tmp / "src")
    shutil.copy2(REPO / "DESIGN.md", tmp / "DESIGN.md")
    subprocess.check_call([str(cam), "--path", str(tmp), "index"], cwd=tmp)
    extra = ["--hash-embed"] if hash_embed else []
    design = (tmp / "DESIGN.md").read_text()
    cam_json(
        cam,
        tmp,
        [
            "add",
            "--summary",
            "cam design: code graph virtual paths, solution tree, Ebbinghaus, no MCP",
            *extra,
        ],
        stdin=design,
    )
    return tmp, CODE_QUESTIONS, dump_rust(tmp)


def summarize_pair(full: dict, cam: dict) -> dict:
    saved = full["total_tokens"] - cam["total_tokens"]
    return {
        "full_accuracy": full["accuracy"],
        "cam_accuracy": cam["accuracy"],
        "full_tokens": full["total_tokens"],
        "cam_tokens": cam["total_tokens"],
        "token_saving": saved,
        "token_saving_pct": (saved / full["total_tokens"]) if full["total_tokens"] else 0.0,
        "full_avg_prompt": full["avg_prompt_tokens"],
        "cam_avg_prompt": cam["avg_prompt_tokens"],
    }


def main() -> int:
    load_dotenv(REPO / ".env")
    p = argparse.ArgumentParser()
    p.add_argument("--track", choices=("solutions", "code", "both"), default="both")
    p.add_argument("--data", type=Path, default=DEFAULT_LIFE)
    p.add_argument("--hash-embed", action="store_true", default=True)
    p.add_argument("--potion", action="store_true", help="use potion embedder instead of hash")
    args = p.parse_args()
    hash_embed = not args.potion
    key, base, model = llm_config()
    cam = cam_bin()

    ping = chat(
        key,
        base,
        model,
        [{"role": "user", "content": "Reply with the single word pong."}],
        max_tokens=16,
    )
    print(f"llm ok  model={ping['raw_model']}  ping_tokens={ping['total_tokens']}", flush=True)

    out_dir = HERE / "results"
    out_dir.mkdir(exist_ok=True)
    tracks = {}
    tmps: list[Path] = []

    try:
        if args.track in {"solutions", "both"}:
            root, questions, dump = setup_solutions(cam, args.data, hash_embed)
            tmps.append(root)
            print("\n== solutions / full dump ==", flush=True)
            full = run_conversation(
                key=key,
                base=base,
                model=model,
                questions=questions,
                mode="full",
                system=(
                    "You answer questions about prior coding-agent sessions. "
                    "Use only the session log.\n\n" + dump
                ),
                retrieve=lambda q: "",
            )
            print("\n== solutions / cam recall ==", flush=True)
            cam_run_ = run_conversation(
                key=key,
                base=base,
                model=model,
                questions=questions,
                mode="cam",
                system="You answer from retrieved session snippets only. If context is missing, say so.",
                retrieve=lambda q: solutions_cam_context(cam, root, q, hash_embed),
            )
            tracks["solutions"] = {
                "full": full,
                "cam": cam_run_,
                "compare": summarize_pair(full, cam_run_),
            }

        if args.track in {"code", "both"}:
            root, questions, dump = setup_code(cam, hash_embed)
            tmps.append(root)
            print("\n== code / full dump ==", flush=True)
            full = run_conversation(
                key=key,
                base=base,
                model=model,
                questions=questions,
                mode="full",
                system=(
                    "You answer questions about this codebase. Use only the sources.\n\n"
                    + dump
                ),
                retrieve=lambda q: "",
            )
            print("\n== code / cam recall+read+ref ==", flush=True)
            cam_run_ = run_conversation(
                key=key,
                base=base,
                model=model,
                questions=questions,
                mode="cam",
                system=(
                    "You answer from retrieved cam recall/read/ref snippets only. "
                    "If context is missing, say so."
                ),
                retrieve=lambda q: code_cam_context(cam, root, q, hash_embed),
            )
            tracks["code"] = {
                "full": full,
                "cam": cam_run_,
                "compare": summarize_pair(full, cam_run_),
            }
    finally:
        for root in tmps:
            shutil.rmtree(root, ignore_errors=True)

    summary = {
        "model": model,
        "resolved_model": ping["raw_model"],
        "embedder": "hash" if hash_embed else "potion-multilingual-128M",
        "tracks": {name: {"compare": t["compare"], "full": {k: v for k, v in t["full"].items() if k != "rows"}, "cam": {k: v for k, v in t["cam"].items() if k != "rows"}} for name, t in tracks.items()},
    }
    stamp = time.strftime("%Y%m%d-%H%M%S")
    payload = {"summary": summary, "tracks": tracks}
    dest = out_dir / f"llm-multiturn-{stamp}.json"
    dest.write_text(json.dumps(payload, indent=2, ensure_ascii=False))
    print("\n" + json.dumps(summary, indent=2))
    print(f"wrote {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
