#!/usr/bin/env python3
"""LongMemCode JSON-over-stdio adapter backed by cam's SQLite graph.

Translation table: SCIP-style ids collected from the scenario file, keyed by
trailing identifier. Graph facts come from cam (index + edges). implementors
are not in cam v1 and return [].
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from graph import CamGraph
from scip import catalog_from_scenarios, path_hint, path_matches, trailing_ident


UNSUPPORTED_ONE_HOP = frozenset({"implementors"})


def map_nodes(nodes, catalog: dict[str, list[str]], path: str = "") -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for n in nodes:
        ids = catalog.get(n.name) or []
        picked = ids
        hint = path or n.file_path.replace("\\", "/")
        hinted = [i for i in ids if path_matches(n.file_path, path_hint(i)) or (hint and hint in i)]
        if hinted:
            picked = hinted
        for sid in picked:
            if sid not in seen:
                seen.add(sid)
                out.append(sid)
    return out


def answer(query: dict, graph: CamGraph, catalog: dict[str, list[str]]) -> list[str]:
    op = query.get("op")
    if op == "lookup":
        name = query.get("name") or ""
        if not query.get("bare_name") and (
            name.startswith("rust-analyzer") or name.startswith("file:")
        ):
            nodes = graph.resolve_stable(name, trailing_ident(name), path_hint(name))
            if nodes:
                return [name]
            return []
        kind = query.get("kind")
        ident = trailing_ident(name) or name
        nodes = graph.nodes_named(ident, kind)
        if not nodes and kind:
            nodes = graph.nodes_named(ident, None)
        return map_nodes(nodes, catalog)

    if op in {"callers", "callees"}:
        sid = query.get("sym_stable_id") or ""
        nodes = graph.resolve_stable(sid, trailing_ident(sid), path_hint(sid))
        direction = "in" if op == "callers" else "out"
        neigh = graph.neighbors([n.id for n in nodes], direction)
        return map_nodes(neigh, catalog)

    if op == "contained_by":
        sid = query.get("sym_stable_id") or ""
        nodes = graph.resolve_stable(sid, trailing_ident(sid), path_hint(sid))
        files = {n.file_path for n in nodes}
        kids: list = []
        for f in files:
            kids.extend(graph.file_symbols(f))
        return map_nodes(kids, catalog, path_hint(sid))

    if op == "file_symbols":
        fp = query.get("file_path") or ""
        nodes = graph.file_symbols(fp)
        ids = map_nodes(nodes, catalog, fp)
        file_id = f"file:{fp}"
        if file_id not in ids:
            ids.insert(0, file_id)
        crate = fp.replace("\\", "/").split("/", 1)[0]
        for sids in catalog.values():
            for sid in sids:
                if sid.endswith("crate/") and crate and crate in sid and sid not in ids:
                    ids.append(sid)
        return ids

    if op == "orphans":
        nodes = graph.orphans(query.get("kind"))
        return map_nodes(nodes, catalog)

    if op == "implementors":
        return []

    return []


def load_catalog(scenario_path: Path | None) -> dict[str, list[str]]:
    if not scenario_path or not scenario_path.exists():
        return {}
    data = json.loads(scenario_path.read_text())
    return catalog_from_scenarios(data)


def main() -> int:
    p = argparse.ArgumentParser(description="cam LongMemCode adapter")
    p.add_argument("--corpus", required=True, help="project root (cam index lives in .cam/)")
    p.add_argument("--db", default="", help="override path to cam.db")
    p.add_argument(
        "--scenarios",
        default="",
        help="scenario JSON used to build SCIP id translation table",
    )
    args = p.parse_args()
    root = Path(args.corpus)
    db = Path(args.db) if args.db else root / ".cam" / "cam.db"
    if not db.exists():
        print(
            json.dumps(
                {"results": [], "cost_usd": 0.0, "error": f"missing {db}; run cam index"}
            ),
            flush=True,
        )
        return 1
    catalog = load_catalog(Path(args.scenarios) if args.scenarios else None)
    graph = CamGraph(db)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            query = req.get("query") or req
            results = answer(query, graph, catalog)
            sys.stdout.write(json.dumps({"results": results, "cost_usd": 0.0}) + "\n")
        except Exception as exc:  # noqa: BLE001 — protocol requires an error object
            sys.stdout.write(
                json.dumps({"results": [], "cost_usd": 0.0, "error": str(exc)}) + "\n"
            )
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
