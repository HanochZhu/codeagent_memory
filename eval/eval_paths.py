"""Corpus and binary locations for the eval scripts, read from eval/datasets.toml."""

from __future__ import annotations

import os
import subprocess
import sys
import tomllib
from pathlib import Path

EVAL = Path(__file__).resolve().parent
REPO = EVAL.parent
CONFIG = EVAL / "datasets.toml"
EXE = "cam.exe" if sys.platform == "win32" else "cam"


def dataset(name: str) -> Path:
    """Configured path for `name`, resolved against the repository root.

    `CAM_EVAL_<NAME>` takes precedence, so a one-off run in another checkout
    does not need the config file edited.
    """
    override = os.environ.get(f"CAM_EVAL_{name.upper()}")
    if override:
        return Path(override).expanduser()

    with CONFIG.open("rb") as handle:
        configured = tomllib.load(handle).get("datasets", {})
    if name not in configured:
        raise KeyError(f"no [datasets].{name} entry in {CONFIG}")

    path = Path(configured[name]).expanduser()
    return path if path.is_absolute() else Path(os.path.normpath(REPO / path))


def cam_binary() -> Path:
    """The `cam` build to benchmark, built on demand.

    `CARGO_TARGET_DIR` comes first because a stale `target/debug/` from an
    earlier layout would otherwise be picked up silently, and a benchmark that
    measures last week's binary is worse than one that fails.
    """
    configured = os.environ.get("CAM_BIN")
    if configured:
        return Path(configured).expanduser()

    target = Path(os.environ.get("CARGO_TARGET_DIR") or REPO / "target")
    built = target / "debug" / EXE
    if built.exists():
        return built

    subprocess.check_call(["cargo", "build", "-q"], cwd=REPO)
    if not built.exists():
        raise FileNotFoundError(f"cargo build did not produce {built}")
    return built
