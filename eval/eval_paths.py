"""Corpus locations for the eval scripts, read from eval/datasets.toml."""

from __future__ import annotations

import os
import tomllib
from pathlib import Path

EVAL = Path(__file__).resolve().parent
REPO = EVAL.parent
CONFIG = EVAL / "datasets.toml"


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
