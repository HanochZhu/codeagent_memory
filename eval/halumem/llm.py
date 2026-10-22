"""DeepSeek chat calls for the HaluMem comparison.

The model name comes from MODEL, then deepseek-flash. The key is read from
.deepseek.env or .env. Thinking is turned off so the judge returns JSON.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


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


def model_name() -> str:
    return os.environ.get("MODEL") or "deepseek-flash"


def base_url() -> str:
    return (
        os.environ.get("DEEPSEEK_BASE_URL")
        or os.environ.get("BASE_URL")
        or "https://api.deepseek.com"
    ).rstrip("/")


def api_key() -> str:
    load_env()
    key = os.environ.get("DEEPSEEK_API_KEY") or ""
    if not key:
        raise SystemExit("DEEPSEEK_API_KEY missing")
    return key


def parse_json_object(content: str) -> dict:
    text = re.sub(r"<think>.*?</think>", "", content or "", flags=re.DOTALL).strip()
    fenced = re.search(r"```json\s*(\{.*?\})\s*```", text, re.DOTALL)
    raw = fenced.group(1) if fenced else ""
    if not raw:
        start = text.find("{")
        end = text.rfind("}")
        if start >= 0 and end > start:
            raw = text[start : end + 1]
    if not raw:
        raise ValueError(f"no JSON object in model output: {text[:300]}")
    parsed = json.loads(raw)
    if not isinstance(parsed, dict):
        raise ValueError("model JSON was not an object")
    return parsed


def chat(messages: list[dict], tools: list[dict] | None = None, max_tokens: int = 2000) -> dict:
    payload = {
        "model": model_name(),
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
            [
                "curl",
                "-sS",
                "-4",
                "--max-time",
                "180",
                f"{base_url()}/chat/completions",
                "-H",
                f"Authorization: Bearer {api_key()}",
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
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


def complete(prompt: str, max_tokens: int = 1500) -> str:
    data = chat([{"role": "user", "content": prompt}], max_tokens=max_tokens)
    message = (data.get("choices") or [{}])[0].get("message") or {}
    return message.get("content") or ""


def complete_json(prompt: str) -> dict:
    last = ""
    for _ in range(3):
        text = complete(prompt)
        last = text
        try:
            return parse_json_object(text)
        except (ValueError, json.JSONDecodeError):
            continue
    raise ValueError(f"judge JSON failed: {last[:300]}")
