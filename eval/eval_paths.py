"""Corpus and binary locations for the eval scripts, read from eval/datasets.toml."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tomllib
from pathlib import Path

EVAL = Path(__file__).resolve().parent
REPO = EVAL.parent
CONFIG = EVAL / "datasets.toml"
EXE = "cam.exe" if sys.platform == "win32" else "cam"
POTION_MODEL = "minishlab/potion-multilingual-128M"


def embed_env(base: dict[str, str] | None = None, *, hash_embed: bool) -> dict[str, str]:
    """Environment for a cam child process.

    Eval recall uses potion-multilingual-128M unless ``hash_embed`` is set.
    ``CAM_REQUIRE_MODEL2VEC`` refuses a silent fallback to the hash embedder.
    """
    env = dict(os.environ if base is None else base)
    if hash_embed:
        env["CAM_HASH_EMBED"] = "1"
        env.pop("CAM_EMBED_MODEL", None)
        env.pop("CAM_REQUIRE_MODEL2VEC", None)
    else:
        env.pop("CAM_HASH_EMBED", None)
        env["CAM_EMBED_MODEL"] = POTION_MODEL
        env["CAM_REQUIRE_MODEL2VEC"] = "1"
    return env


def apply_embed_env(hash_embed: bool) -> None:
    """Install `embed_env` on this process so child `cam` commands inherit it."""
    chosen = embed_env(hash_embed=hash_embed)
    for key in ("CAM_HASH_EMBED", "CAM_EMBED_MODEL", "CAM_REQUIRE_MODEL2VEC"):
        if key in chosen:
            os.environ[key] = chosen[key]
        else:
            os.environ.pop(key, None)


def add_hash_embed_argument(parser: argparse.ArgumentParser) -> None:
    """Second eval path. The default path loads the embedding model and errors on failure."""
    parser.add_argument(
        "--hash-embed",
        action="store_true",
        help=(
            f"use the hash embedder for recall vectors; "
            f"the default loads {POTION_MODEL} and errors if that model fails"
        ),
    )


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
