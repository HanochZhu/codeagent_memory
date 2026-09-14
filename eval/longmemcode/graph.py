"""Read cam's SQLite graph for LongMemCode queries."""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass
from pathlib import Path

from scip import path_matches


@dataclass
class Node:
    id: str
    kind: str
    name: str
    file_path: str
    start_line: int
    signature: str | None


class CamGraph:
    def __init__(self, db_path: Path):
        self.conn = sqlite3.connect(db_path)
        self.conn.row_factory = sqlite3.Row

    def nodes_named(self, name: str, kind: str | None = None) -> list[Node]:
        if kind:
            kinds = _cam_kinds(kind)
            q = f"SELECT * FROM nodes WHERE name = ? AND kind IN ({','.join('?' * len(kinds))})"
            rows = self.conn.execute(q, [name, *kinds]).fetchall()
        else:
            rows = self.conn.execute(
                "SELECT * FROM nodes WHERE name = ? AND kind != 'file'", [name]
            ).fetchall()
        return [_node(r) for r in rows]

    def node_by_id(self, nid: str) -> Node | None:
        row = self.conn.execute("SELECT * FROM nodes WHERE id = ?", [nid]).fetchone()
        return _node(row) if row else None

    def file_symbols(self, file_path: str) -> list[Node]:
        rows = self.conn.execute(
            "SELECT * FROM nodes WHERE file_path = ? AND kind != 'file' ORDER BY start_line",
            [file_path],
        ).fetchall()
        if rows:
            return [_node(r) for r in rows]
        rows = self.conn.execute(
            "SELECT * FROM nodes WHERE file_path LIKE ? AND kind != 'file' ORDER BY start_line",
            [f"%/{file_path}"],
        ).fetchall()
        return [_node(r) for r in rows]

    def neighbors(self, node_ids: list[str], direction: str) -> list[Node]:
        if not node_ids:
            return []
        placeholders = ",".join("?" * len(node_ids))
        if direction == "in":
            sql = f"""
                SELECT DISTINCT n.* FROM edges e
                JOIN nodes n ON n.id = e.source
                WHERE e.target IN ({placeholders}) AND e.kind = 'calls'
            """
        else:
            sql = f"""
                SELECT DISTINCT n.* FROM edges e
                JOIN nodes n ON n.id = e.target
                WHERE e.source IN ({placeholders}) AND e.kind = 'calls'
            """
        return [_node(r) for r in self.conn.execute(sql, node_ids).fetchall()]

    def orphans(self, kind: str | None = None) -> list[Node]:
        kinds = _cam_kinds(kind) if kind else ("function", "method")
        q = f"""
            SELECT n.* FROM nodes n
            WHERE n.kind IN ({",".join("?" * len(kinds))})
              AND NOT EXISTS (
                SELECT 1 FROM edges e
                WHERE e.target = n.id AND e.kind = 'calls'
              )
        """
        return [_node(r) for r in self.conn.execute(q, kinds).fetchall()]

    def resolve_stable(self, stable_id: str, trailing: str, path_hint: str) -> list[Node]:
        nodes = self.nodes_named(trailing)
        if path_hint:
            hinted = [n for n in nodes if path_matches(n.file_path, path_hint)]
            if hinted:
                return hinted
        return nodes


def _node(row: sqlite3.Row) -> Node:
    return Node(
        id=row["id"],
        kind=row["kind"],
        name=row["name"],
        file_path=row["file_path"],
        start_line=row["start_line"],
        signature=row["signature"],
    )


def _cam_kinds(kind: str | None) -> tuple[str, ...]:
    if not kind:
        return ("function", "method", "class", "struct")
    k = kind.lower()
    if k in {"struct", "class", "type", "enum", "trait"}:
        return ("struct", "class")
    if k in {"function", "fn", "method"}:
        return ("function", "method")
    return (k,)
