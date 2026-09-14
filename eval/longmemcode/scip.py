"""Parse LongMemCode / rust-analyzer-style stable ids."""

from __future__ import annotations

import re

_MARKERS = "().#:/;[]`"


def trailing_ident(stable_id: str) -> str:
    trimmed = stable_id.rstrip(_MARKERS)
    i = len(trimmed)
    while i > 0 and (trimmed[i - 1].isalnum() or trimmed[i - 1] == "_"):
        i -= 1
    return trimmed[i:]


def path_hint(stable_id: str) -> str:
    """Best-effort repo-relative path fragment inside a SCIP id."""
    if stable_id.startswith("file:"):
        return stable_id[5:]
    parts = stable_id.split()
    if not parts:
        return ""
    rest = parts[-1]
    rest = rest.split("#", 1)[0]
    rest = rest.split(".", 1)[0]
    rest = re.sub(r"/impl#.*$", "", rest)
    rest = re.sub(r"/impl$", "", rest)
    rest = rest.replace("`", "")
    return rest.strip("/")


def path_matches(file_path: str, hint: str) -> bool:
    """True if a cam file_path is the same module as a SCIP path fragment."""
    fp = file_path.replace("\\", "/").lower().lstrip("./")
    h = hint.strip("/").lower()
    if not fp or not h:
        return False
    if h in fp or fp in h:
        return True
    segs = [s for s in h.split("/") if s and s != "crate"]
    if not segs:
        return False
    # rust-analyzer: builder/command/Command → clap_builder/src/builder/command.rs
    if segs[-1].endswith(".rs") or segs[-1].endswith(".py"):
        return segs[-1] in fp
    module = "/".join(segs[:-1]) if len(segs) > 1 else ""
    if module and module in fp:
        return True
    stem = segs[-2] if len(segs) > 1 else segs[0]
    return f"/{stem}." in f"/{fp}" or fp.endswith(f"/{stem}.rs") or fp.endswith(f"/{stem}.py")


def collect_ids(scenarios: list[dict]) -> list[str]:
    ids: list[str] = []
    for sc in scenarios:
        q = sc.get("query") or {}
        if "sym_stable_id" in q:
            ids.append(q["sym_stable_id"])
        if q.get("op") == "lookup" and not q.get("bare_name"):
            name = q.get("name") or ""
            if name.startswith("rust-analyzer") or name.startswith("file:"):
                ids.append(name)
        exp = sc.get("expected") or {}
        for key in ("required", "stable_ids"):
            ids.extend(exp.get(key) or [])
        if exp.get("stable_id"):
            ids.append(exp["stable_id"])
    # preserve order, unique
    seen: set[str] = set()
    out: list[str] = []
    for i in ids:
        if i and i not in seen:
            seen.add(i)
            out.append(i)
    return out


def index_catalog(ids: list[str]) -> dict[str, list[str]]:
    by_name: dict[str, list[str]] = {}
    for sid in ids:
        name = trailing_ident(sid)
        if not name:
            continue
        by_name.setdefault(name, []).append(sid)
    return by_name


def _add(by_name: dict[str, list[str]], name: str, sid: str) -> None:
    if not name or not sid:
        return
    bucket = by_name.setdefault(name, [])
    if sid not in bucket:
        bucket.append(sid)


def catalog_from_scenarios(scenarios: list[dict]) -> dict[str, list[str]]:
    """Name → SCIP ids. Harvest trailing idents plus each lookup's query name.

    Query-name harvest is how `local N` gold (no ident in the id) gets keyed.
    """
    by_name: dict[str, list[str]] = {}
    for sid in collect_ids(scenarios):
        _add(by_name, trailing_ident(sid), sid)
        if sid.startswith("file:"):
            _add(by_name, file_name(sid[5:]), sid)

    for sc in scenarios:
        q = sc.get("query") or {}
        exp = sc.get("expected") or {}
        ids = list(exp.get("required") or []) + list(exp.get("stable_ids") or [])
        if exp.get("stable_id"):
            ids.append(exp["stable_id"])
        qname = q.get("name") or ""
        ident = trailing_ident(qname) or qname
        if ident:
            for sid in ids:
                _add(by_name, ident, sid)
        fp = q.get("file_path") or ""
        if fp:
            for sid in ids:
                _add(by_name, file_name(fp), sid)
    return by_name


def file_name(path: str) -> str:
    return path.replace("\\", "/").rstrip("/").rsplit("/", 1)[-1]
