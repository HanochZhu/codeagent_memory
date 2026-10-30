"""Fetch the write-memory case set. The notes live in their own repository."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

from eval_paths import dataset  # noqa: E402

CASES_URL = "https://raw.githubusercontent.com/HanochZhu/cam-write-memory-eval/main/cases.json"


def cases_path() -> Path:
    return dataset("write_memory")


def ensure_cases() -> Path:
    """Download cases.json on first use. A later run reuses the local file."""
    path = cases_path()
    if path.exists() and path.stat().st_size > 100:
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_suffix(path.suffix + ".partial")
    subprocess.check_call(
        ["curl", "-fsSL", "--max-time", "60", "-o", str(partial), CASES_URL],
    )
    partial.replace(path)
    return path
