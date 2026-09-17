"""Read codegraph's SQLite graph for LongMemCode queries.

Same Node shape as CamGraph. Callers use `calls` + `references`; implementors
use `implements` / `overrides` / `extends`.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path

from graph import Node, _node
from scip import path_matches

CALLER_KINDS = ("calls", "references")
IMPL_KINDS = ("implements", "overrides", "extends")


class CodegraphGraph:
    def __init__(self, db_path: Path, root: Path | None = None):
        self.conn = sqlite3.connect(db_path)
        self.conn.row_factory = sqlite3.Row
        self.root = root.resolve() if root else None

    def _rel(self, file_path: str) -> str:
        fp = file_path.replace("\\", "/")
        if self.root:
            root = str(self.root).replace("\\", "/").rstrip("/")
            if fp.startswith(root + "/"):
                return fp[len(root) + 1 :]
        return fp

    def _nodes(self, rows) -> list[Node]:
        out = []
        for r in rows:
            n = _node(r)
            n.file_path = self._rel(n.file_path)
            out.append(n)
        return out

    def nodes_named(self, name: str, kind: str | None = None) -> list[Node]:
        if kind:
            kinds = _cg_kinds(kind)
            q = f"SELECT * FROM nodes WHERE name = ? AND kind IN ({','.join('?' * len(kinds))})"
            rows = self.conn.execute(q, [name, *kinds]).fetchall()
        else:
            rows = self.conn.execute(
                "SELECT * FROM nodes WHERE name = ? AND kind != 'file'", [name]
            ).fetchall()
        return self._nodes(rows)

    def node_by_id(self, nid: str) -> Node | None:
        row = self.conn.execute("SELECT * FROM nodes WHERE id = ?", [nid]).fetchone()
        if not row:
            return None
        return self._nodes([row])[0]

    def file_symbols(self, file_path: str) -> list[Node]:
        rows = self.conn.execute(
            "SELECT * FROM nodes WHERE file_path = ? AND kind != 'file' ORDER BY start_line",
            [file_path],
        ).fetchall()
        if not rows and self.root:
            rows = self.conn.execute(
                "SELECT * FROM nodes WHERE file_path = ? AND kind != 'file' ORDER BY start_line",
                [str(self.root / file_path)],
            ).fetchall()
        if not rows:
            rows = self.conn.execute(
                "SELECT * FROM nodes WHERE file_path LIKE ? AND kind != 'file' ORDER BY start_line",
                [f"%/{file_path}"],
            ).fetchall()
        return self._nodes(rows)

    def neighbors(self, node_ids: list[str], direction: str) -> list[Node]:
        if not node_ids:
            return []
        placeholders = ",".join("?" * len(node_ids))
        kinds = ",".join("?" * len(CALLER_KINDS))
        if direction == "in":
            sql = f"""
                SELECT DISTINCT n.* FROM edges e
                JOIN nodes n ON n.id = e.source
                WHERE e.target IN ({placeholders}) AND e.kind IN ({kinds})
            """
        else:
            sql = f"""
                SELECT DISTINCT n.* FROM edges e
                JOIN nodes n ON n.id = e.target
                WHERE e.source IN ({placeholders}) AND e.kind IN ({kinds})
            """
        return self._nodes(self.conn.execute(sql, [*node_ids, *CALLER_KINDS]).fetchall())

    def members(self, node_ids: list[str]) -> list[Node]:
        if not node_ids:
            return []
        placeholders = ",".join("?" * len(node_ids))
        rows = self.conn.execute(
            f"""
            SELECT DISTINCT n.* FROM edges e
            JOIN nodes n ON n.id = e.target
            WHERE e.source IN ({placeholders}) AND e.kind = 'contains'
            """,
            node_ids,
        ).fetchall()
        kids = self._nodes(rows)
        if kids:
            return kids
        files = {n.file_path for nid in node_ids if (n := self.node_by_id(nid))}
        out: list[Node] = []
        for f in files:
            out.extend(self.file_symbols(f))
        return out

    def implementors(self, node_ids: list[str]) -> list[Node]:
        if not node_ids:
            return []
        placeholders = ",".join("?" * len(node_ids))
        kinds = ",".join("?" * len(IMPL_KINDS))
        rows = self.conn.execute(
            f"""
            SELECT DISTINCT n.* FROM edges e
            JOIN nodes n ON n.id = e.source
            WHERE e.target IN ({placeholders}) AND e.kind IN ({kinds})
            """,
            [*node_ids, *IMPL_KINDS],
        ).fetchall()
        return self._nodes(rows)

    def orphans(self, kind: str | None = None) -> list[Node]:
        kinds = _cg_kinds(kind) if kind else ("function", "method")
        q = f"""
            SELECT n.* FROM nodes n
            WHERE n.kind IN ({",".join("?" * len(kinds))})
              AND NOT EXISTS (
                SELECT 1 FROM edges e
                WHERE e.target = n.id AND e.kind IN ('calls', 'references')
              )
        """
        return self._nodes(self.conn.execute(q, kinds).fetchall())

    def resolve_stable(self, stable_id: str, trailing: str, hint: str) -> list[Node]:
        del stable_id
        nodes = self.nodes_named(trailing)
        if hint:
            hinted = [n for n in nodes if path_matches(n.file_path, hint)]
            if hinted:
                return hinted
        return nodes


def _cg_kinds(kind: str | None) -> tuple[str, ...]:
    if not kind:
        return (
            "function",
            "method",
            "class",
            "struct",
            "type",
            "type_alias",
            "enum",
            "enum_member",
            "trait",
            "interface",
        )
    k = kind.lower()
    if k in {"struct", "class", "type", "enum", "trait", "interface"}:
        return (
            "struct",
            "class",
            "type",
            "type_alias",
            "enum",
            "enum_member",
            "trait",
            "interface",
        )
    if k in {"function", "fn", "method"}:
        return ("function", "method")
    return (k,)
