use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{bail, Result};
use clap::ValueEnum;
use rusqlite::Connection;
use serde::Serialize;

use super::index::{normalize_virt, virt_parts};
use crate::db;
use crate::project::Project;

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RefDir {
    In,
    Out,
}

#[derive(Debug, Serialize)]
pub struct LsEntry {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ReadResult {
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<i64>,
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct RefHit {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub start_line: i64,
    pub kind: String,
    pub line: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RefResult {
    pub symbol: String,
    pub direction: RefDir,
    pub refs: Vec<RefHit>,
}

pub fn ls(project: &Project, virt_path: Option<&str>) -> Result<Vec<LsEntry>> {
    let conn = db::open_db(&project.db_path())?;
    let virt = normalize_virt(virt_path);
    let (file, symbol) = virt_parts(&virt);

    if symbol.is_some() {
        bail!("`cam ls` does not list inside a symbol; use `cam read {virt}`");
    }

    if let Some(file) = file {
        if looks_indexed_file(&conn, file)? {
            return list_symbols(&conn, file);
        }
        return list_prefix(&conn, file);
    }
    list_prefix(&conn, "")
}

pub fn read(project: &Project, virt_path: &str, full: bool) -> Result<ReadResult> {
    let conn = db::open_db(&project.db_path())?;
    let virt = normalize_virt(Some(virt_path));
    let (file, symbol) = virt_parts(&virt);
    let Some(file) = file else {
        bail!("specify a file or symbol path, e.g. src/main.rs or src/main.rs/main");
    };

    if let Some(symbol) = symbol {
        return read_symbol(&conn, &project.root, file, symbol);
    }
    if full {
        let abs = project.root.join(file);
        let source = fs::read_to_string(&abs)?;
        return Ok(ReadResult {
            kind: "file".into(),
            path: file.to_string(),
            start_line: Some(1),
            end_line: Some(source.lines().count() as i64),
            source,
        });
    }
    Ok(ReadResult {
        kind: "outline".into(),
        path: file.to_string(),
        start_line: None,
        end_line: None,
        source: outline_text(&conn, file)?,
    })
}

pub fn refs(project: &Project, symbol: &str, dir: RefDir) -> Result<RefResult> {
    let conn = db::open_db(&project.db_path())?;
    let nodes = resolve_symbols(&conn, symbol)?;
    if nodes.is_empty() {
        bail!("symbol not found: {symbol}");
    }
    if nodes.len() > 1 {
        let listed = nodes
            .iter()
            .map(|(id, _, path, line)| format!("{id} ({path}:{line})"))
            .collect::<Vec<_>>()
            .join(", ");
        bail!("ambiguous symbol `{symbol}`: {listed}");
    }
    let (id, name, _, _) = &nodes[0];
    let sql = match dir {
        RefDir::In => {
            "SELECT n.id, n.name, n.file_path, n.start_line, n.kind, e.line
             FROM edges e JOIN nodes n ON n.id = e.source
             WHERE e.target = ?1 AND e.kind IN ('calls', 'references', 'implements')"
        }
        RefDir::Out => {
            "SELECT n.id, n.name, n.file_path, n.start_line, n.kind, e.line
             FROM edges e JOIN nodes n ON n.id = e.target
             WHERE e.source = ?1 AND e.kind IN ('calls', 'references', 'implements')"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let refs = stmt
        .query_map([id], |row| {
            Ok(RefHit {
                id: row.get(0)?,
                name: row.get(1)?,
                file_path: row.get(2)?,
                start_line: row.get(3)?,
                kind: row.get(4)?,
                line: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(RefResult {
        symbol: name.clone(),
        direction: dir,
        refs,
    })
}

fn looks_indexed_file(conn: &Connection, file: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files WHERE path = ?1",
        [file],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn list_symbols(conn: &Connection, file: &str) -> Result<Vec<LsEntry>> {
    let mut stmt = conn.prepare(
        "SELECT name, kind, file_path, start_line, end_line FROM nodes
         WHERE file_path = ?1 AND kind != 'file'
         ORDER BY start_line",
    )?;
    let rows = stmt.query_map([file], |row| {
        let name: String = row.get(0)?;
        let file_path: String = row.get(2)?;
        Ok(LsEntry {
            name: name.clone(),
            kind: row.get(1)?,
            path: Some(format!("{file_path}/{name}")),
            start_line: Some(row.get(3)?),
            end_line: Some(row.get(4)?),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn list_prefix(conn: &Connection, prefix: &str) -> Result<Vec<LsEntry>> {
    let mut stmt = conn.prepare("SELECT path FROM files ORDER BY path")?;
    let paths = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let prefix = prefix.trim_matches('/');
    let mut dirs = BTreeSet::new();
    let mut files = BTreeSet::new();
    for path in paths {
        let rel = if prefix.is_empty() {
            path.as_str()
        } else if path == prefix {
            continue;
        } else if let Some(rest) = path.strip_prefix(&format!("{prefix}/")) {
            rest
        } else {
            continue;
        };
        match rel.split_once('/') {
            Some((dir, _)) => {
                dirs.insert(dir.to_string());
            }
            None => {
                files.insert(rel.to_string());
            }
        }
    }

    let mut entries = Vec::new();
    for dir in dirs {
        let path = if prefix.is_empty() {
            dir.clone()
        } else {
            format!("{prefix}/{dir}")
        };
        entries.push(LsEntry {
            name: dir,
            kind: "dir".into(),
            path: Some(path),
            start_line: None,
            end_line: None,
        });
    }
    for file in files {
        let path = if prefix.is_empty() {
            file.clone()
        } else {
            format!("{prefix}/{file}")
        };
        entries.push(LsEntry {
            name: file,
            kind: "file".into(),
            path: Some(path),
            start_line: None,
            end_line: None,
        });
    }
    Ok(entries)
}

fn outline_text(conn: &Connection, file: &str) -> Result<String> {
    let entries = list_symbols(conn, file)?;
    if entries.is_empty() {
        return Ok(format!("{file}: no symbols"));
    }
    let mut lines = vec![file.to_string()];
    for e in entries {
        let span = match (e.start_line, e.end_line) {
            (Some(s), Some(t)) => format!("{s}-{t}"),
            _ => "?".into(),
        };
        lines.push(format!("  {}  {}  {}", e.name, e.kind, span));
    }
    Ok(lines.join("\n"))
}

fn read_symbol(
    conn: &Connection,
    root: &Path,
    file: &str,
    symbol: &str,
) -> Result<ReadResult> {
    let mut stmt = conn.prepare(
        "SELECT id, name, start_line, end_line, kind FROM nodes
         WHERE file_path = ?1 AND name = ?2 AND kind != 'file'",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![file, symbol], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        bail!("symbol `{symbol}` not found in {file}");
    }
    if rows.len() > 1 {
        let listed = rows
            .iter()
            .map(|(_, _, line, _, kind)| format!("{kind}:{line}"))
            .collect::<Vec<_>>()
            .join(", ");
        bail!("ambiguous symbol `{symbol}` in {file}: {listed}");
    }
    let (_, name, start, end, kind) = &rows[0];
    let source = slice_file(&root.join(file), *start, *end)?;
    Ok(ReadResult {
        kind: kind.clone(),
        path: format!("{file}/{name}"),
        start_line: Some(*start),
        end_line: Some(*end),
        source,
    })
}

fn slice_file(path: &Path, start: i64, end: i64) -> Result<String> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    let start_idx = (start.max(1) as usize).saturating_sub(1);
    let end_idx = (end as usize).min(lines.len());
    Ok(lines[start_idx..end_idx].join("\n"))
}

fn resolve_symbols(conn: &Connection, symbol: &str) -> Result<Vec<(String, String, String, i64)>> {
    if let (Some(file), Some(name)) = virt_parts(&normalize_virt(Some(symbol))) {
        if file.contains('.') {
            let mut stmt = conn.prepare(
                "SELECT id, name, file_path, start_line FROM nodes
                 WHERE file_path = ?1 AND name = ?2 AND kind != 'file'",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![file, name], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !rows.is_empty() {
                return Ok(rows);
            }
        }
    }

    let mut stmt = conn.prepare(
        "SELECT id, name, file_path, start_line FROM nodes
         WHERE (name = ?1 OR id = ?1) AND kind != 'file'",
    )?;
    let rows = stmt
        .query_map([symbol], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
