use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

pub const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL,
    language TEXT NOT NULL,
    size INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    signature TEXT
);

CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(file_path);
CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);
CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);

CREATE TABLE IF NOT EXISTS edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    target TEXT NOT NULL,
    kind TEXT NOT NULL,
    line INTEGER,
    FOREIGN KEY (source) REFERENCES nodes(id) ON DELETE CASCADE,
    FOREIGN KEY (target) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_identity
    ON edges(source, target, kind, IFNULL(line, -1));
CREATE INDEX IF NOT EXISTS idx_edges_source_kind ON edges(source, kind);
CREATE INDEX IF NOT EXISTS idx_edges_target_kind ON edges(target, kind);

CREATE TABLE IF NOT EXISTS solutions (
    id TEXT PRIMARY KEY,
    parent_id TEXT,
    summary TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    recalled_at INTEGER,
    stability REAL NOT NULL DEFAULT 7.0,
    embedding BLOB,
    fts_text TEXT NOT NULL,
    FOREIGN KEY (parent_id) REFERENCES solutions(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_solutions_parent ON solutions(parent_id);

CREATE VIRTUAL TABLE IF NOT EXISTS solutions_fts USING fts5(
    summary,
    fts_text,
    content='solutions',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS solutions_ai AFTER INSERT ON solutions BEGIN
    INSERT INTO solutions_fts(rowid, summary, fts_text)
    VALUES (NEW.rowid, NEW.summary, NEW.fts_text);
END;

CREATE TRIGGER IF NOT EXISTS solutions_ad AFTER DELETE ON solutions BEGIN
    INSERT INTO solutions_fts(solutions_fts, rowid, summary, fts_text)
    VALUES ('delete', OLD.rowid, OLD.summary, OLD.fts_text);
END;

CREATE TRIGGER IF NOT EXISTS solutions_au AFTER UPDATE ON solutions BEGIN
    INSERT INTO solutions_fts(solutions_fts, rowid, summary, fts_text)
    VALUES ('delete', OLD.rowid, OLD.summary, OLD.fts_text);
    INSERT INTO solutions_fts(rowid, summary, fts_text)
    VALUES (NEW.rowid, NEW.summary, NEW.fts_text);
END;
"#;

pub fn open_db(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path).with_context(|| format!("open sqlite {}", path.display()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "solutions", "recalled_at")? {
        conn.execute("ALTER TABLE solutions ADD COLUMN recalled_at INTEGER", [])?;
    }
    if !column_exists(conn, "solutions", "stability")? {
        conn.execute(
            "ALTER TABLE solutions ADD COLUMN stability REAL NOT NULL DEFAULT 7.0",
            [],
        )?;
    }
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn table_count(conn: &Connection, table: &str) -> Result<i64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .with_context(|| format!("count {table}"))
}

pub fn node_exists(conn: &Connection, id: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT 1 FROM nodes WHERE id = ?1", [id], |row| row.get(0))
        .optional()?;
    Ok(found.is_some())
}
