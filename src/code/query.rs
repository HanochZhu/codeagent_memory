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

/// `cam read` either returns content or, when a symbol name occurs more than
/// once in the file, the candidates so the caller can re-ask with a node id.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ReadOutcome {
    Ok(ReadResult),
    Ambiguous(Ambiguous),
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

/// The node `cam ref` settled on, so callers can tell which same-named
/// definition the edges belong to.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
}

#[derive(Debug, Serialize)]
pub struct RefResult {
    pub symbol: String,
    pub direction: RefDir,
    pub resolved: ResolvedSymbol,
    pub refs: Vec<RefHit>,
}

/// One same-named definition, ranked by how well it matches the hints.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolCandidate {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub score: f64,
}

/// Returned instead of an error when a bare name matches several nodes.
/// Re-call with `id`, or narrow with the `file` / `kind` / `scope` hints.
#[derive(Debug, Serialize)]
pub struct Ambiguous {
    pub symbol: String,
    pub total_candidates: usize,
    pub candidates: Vec<SymbolCandidate>,
    pub message: String,
}

/// `cam ref` either resolves to exactly one node or reports the candidates.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum RefOutcome {
    Ok(RefResult),
    Ambiguous(Ambiguous),
}

/// Optional narrowing for symbol resolution. All given hints must match.
#[derive(Debug, Clone, Copy, Default)]
pub struct SymbolHints<'a> {
    /// Case-insensitive substring of `file_path` (e.g. `config.ts` or `core/src/config`).
    pub file: Option<&'a str>,
    /// Exact node kind: function, method, struct, class, trait, enum, enum_variant, type_alias.
    pub kind: Option<&'a str>,
    /// Sub-directory prefix relative to the project root, typically a nested
    /// repository listed as `project` by `cam ls` (e.g. `codely-cli`).
    pub scope: Option<&'a str>,
}

impl SymbolHints<'_> {
    fn any(&self) -> bool {
        self.file.is_some() || self.kind.is_some() || self.scope.is_some()
    }
}

const MAX_CANDIDATES: usize = 20;

pub fn ls(project: &Project, virt_path: Option<&str>) -> Result<Vec<LsEntry>> {
    let conn = project.connect()?;
    ensure_graph(project, &conn)?;
    let virt = normalize_virt(virt_path);
    let (file, symbol) = virt_parts(&virt);

    if symbol.is_some() {
        bail!("`cam ls` does not list inside a symbol; use `cam read {virt}`");
    }

    if let Some(file) = file {
        if looks_indexed_file(&conn, file)? {
            return list_symbols(&conn, file);
        }
        return list_prefix(&conn, &project.root, file);
    }
    list_prefix(&conn, &project.root, "")
}

pub fn read(project: &Project, virt_path: &str, full: bool) -> Result<ReadOutcome> {
    let conn = project.connect()?;
    let virt = normalize_virt(Some(virt_path));

    // A node id from a previous ambiguous answer is the zero-ambiguity form.
    if let Some(node) = node_by_id(&conn, &virt)? {
        let source = slice_file(
            &project.root.join(&node.file_path),
            node.start_line,
            node.end_line,
        )?;
        return Ok(ReadOutcome::Ok(ReadResult {
            kind: node.kind,
            path: format!("{}/{}", node.file_path, node.name),
            start_line: Some(node.start_line),
            end_line: Some(node.end_line),
            source,
        }));
    }

    let (file, symbol) = virt_parts(&virt);
    let Some(file) = file else {
        bail!("specify a file or symbol path, e.g. src/main.rs or src/main.rs/main");
    };
    if !full || symbol.is_some() {
        ensure_graph(project, &conn)?;
    }

    if let Some(symbol) = symbol {
        return read_symbol(&conn, &project.root, file, symbol);
    }
    if full {
        let abs = project.root.join(file);
        let source = fs::read_to_string(&abs)?;
        return Ok(ReadOutcome::Ok(ReadResult {
            kind: "file".into(),
            path: file.to_string(),
            start_line: Some(1),
            end_line: Some(source.lines().count() as i64),
            source,
        }));
    }
    Ok(ReadOutcome::Ok(ReadResult {
        kind: "outline".into(),
        path: file.to_string(),
        start_line: None,
        end_line: None,
        source: outline_text(&conn, file)?,
    }))
}

pub fn refs(
    project: &Project,
    symbol: &str,
    dir: RefDir,
    hints: SymbolHints<'_>,
) -> Result<RefOutcome> {
    let conn = project.connect()?;
    ensure_graph(project, &conn)?;
    let nodes = resolve_symbols(&conn, symbol)?;
    if nodes.is_empty() {
        bail!("symbol not found: {symbol}");
    }
    let node = match pick_candidate(symbol, nodes, hints) {
        Resolution::One(node) => node,
        Resolution::Ambiguous(ambiguous) => return Ok(RefOutcome::Ambiguous(ambiguous)),
    };
    let id = &node.id;
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
    let mut refs = stmt
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
    if let Some(prefix) = hints.scope.map(normalize_prefix).filter(|p| !p.is_empty()) {
        refs.retain(|hit| path_in_project(&hit.file_path, &prefix));
    }
    Ok(RefOutcome::Ok(RefResult {
        symbol: node.name.clone(),
        direction: dir,
        resolved: ResolvedSymbol {
            id: node.id,
            name: node.name,
            kind: node.kind,
            file_path: node.file_path,
            start_line: node.start_line,
        },
        refs,
    }))
}

#[derive(Debug, Clone)]
struct NodeRow {
    id: String,
    name: String,
    kind: String,
    file_path: String,
    start_line: i64,
    end_line: i64,
}

enum Resolution {
    One(NodeRow),
    Ambiguous(Ambiguous),
}

/// Apply the hints to same-named nodes. With no hints, more than one node is
/// ambiguous. With hints, exactly one node matching all of them resolves;
/// zero matches reports every node (the hints were wrong), several matches
/// report only those (the hints were not specific enough).
fn pick_candidate(symbol: &str, nodes: Vec<NodeRow>, hints: SymbolHints<'_>) -> Resolution {
    if nodes.len() == 1 {
        return Resolution::One(nodes.into_iter().next().unwrap());
    }
    let mut scored: Vec<(NodeRow, f64, bool)> = nodes
        .into_iter()
        .map(|node| {
            let (score, matched) = score_candidate(&node, hints);
            (node, score, matched)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.file_path.len().cmp(&b.0.file_path.len()))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });

    let matched: Vec<usize> = scored
        .iter()
        .enumerate()
        .filter(|(_, (_, _, m))| *m)
        .map(|(i, _)| i)
        .collect();
    if hints.any() && matched.len() == 1 {
        return Resolution::One(scored.swap_remove(matched[0]).0);
    }

    let (pool, message): (Vec<_>, String) = if hints.any() && matched.is_empty() {
        (
            scored,
            format!(
                "ambiguous symbol `{symbol}`: no candidate matched the hints; re-call with `id`, or fix `file` / `kind` / `scope`"
            ),
        )
    } else if hints.any() {
        (
            scored.into_iter().filter(|(_, _, m)| *m).collect(),
            format!(
                "ambiguous symbol `{symbol}`: several candidates match the hints; re-call with `id`, or add a more specific `file` / `kind`"
            ),
        )
    } else {
        (
            scored,
            format!(
                "ambiguous symbol `{symbol}`: several definitions share this name; re-call with `id`, or narrow with `file` / `kind` / `scope`"
            ),
        )
    };
    let total_candidates = pool.len();
    let candidates = pool
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(node, score, _)| SymbolCandidate {
            id: node.id,
            name: node.name,
            kind: node.kind,
            file_path: node.file_path,
            start_line: node.start_line,
            end_line: node.end_line,
            score,
        })
        .collect();
    Resolution::Ambiguous(Ambiguous {
        symbol: symbol.to_string(),
        total_candidates,
        candidates,
        message,
    })
}

/// Score in [0, 1]: base 0.5, +0.4 file hint, +0.2 kind hint, +0.1 scope
/// hint, and a small kind-priority bonus when no kind hint was given so the
/// list has a stable, meaningful order. Returns the score and whether every
/// given hint matched.
fn score_candidate(node: &NodeRow, hints: SymbolHints<'_>) -> (f64, bool) {
    let mut score = 0.5;
    let mut all = true;
    if let Some(file) = hints.file {
        let needle = file.trim_matches('/').to_ascii_lowercase();
        if !needle.is_empty() && node.file_path.to_ascii_lowercase().contains(&needle) {
            score += 0.4;
        } else {
            all = false;
        }
    }
    match hints.kind {
        Some(kind) => {
            if node.kind.eq_ignore_ascii_case(kind.trim()) {
                score += 0.2;
            } else {
                all = false;
            }
        }
        None => score += kind_priority(&node.kind),
    }
    if let Some(scope) = hints.scope {
        let prefix = normalize_prefix(scope);
        if prefix.is_empty() || path_in_project(&node.file_path, &prefix) {
            score += 0.1;
        } else {
            all = false;
        }
    }
    (score.min(1.0), all)
}

fn kind_priority(kind: &str) -> f64 {
    match kind {
        "struct" | "class" | "trait" | "enum" | "type_alias" => 0.10,
        "function" => 0.06,
        "method" => 0.04,
        _ => 0.02,
    }
}

fn normalize_prefix(scope: &str) -> String {
    normalize_virt(Some(scope)).replace('\\', "/")
}

fn path_in_project(file_path: &str, prefix: &str) -> bool {
    file_path == prefix || file_path.starts_with(&format!("{prefix}/"))
}

fn ensure_graph(project: &Project, conn: &Connection) -> Result<()> {
    if db::table_count(conn, "files")? > 0 {
        return Ok(());
    }
    if !super::index::project_has_source_files(project)? {
        return Ok(());
    }
    bail!(
        "code graph not indexed for {}; run `cam index` (MCP cam_index) once, then retry",
        project.root.display()
    )
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

/// Directories that are git checkouts of their own are listed as `project`
/// rather than `dir`, so an agent working in an umbrella folder can see the
/// repository boundaries and pass them as the `scope` hint to `cam ref`.
fn dir_kind(root: &Path, rel: &str) -> &'static str {
    if root.join(rel).join(".git").exists() {
        "project"
    } else {
        "dir"
    }
}

fn list_prefix(conn: &Connection, root: &Path, prefix: &str) -> Result<Vec<LsEntry>> {
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
            kind: dir_kind(root, &path).into(),
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

fn read_symbol(conn: &Connection, root: &Path, file: &str, symbol: &str) -> Result<ReadOutcome> {
    let rows = nodes_in_file(conn, file, symbol)?;
    if rows.is_empty() {
        bail!("symbol `{symbol}` not found in {file}");
    }
    let node = match pick_candidate(&format!("{file}/{symbol}"), rows, SymbolHints::default()) {
        Resolution::One(node) => node,
        Resolution::Ambiguous(mut ambiguous) => {
            ambiguous.message = format!(
                "ambiguous symbol `{symbol}` in {file}: several definitions share this name; re-call cam read with one of the candidate ids"
            );
            return Ok(ReadOutcome::Ambiguous(ambiguous));
        }
    };
    let source = slice_file(&root.join(file), node.start_line, node.end_line)?;
    Ok(ReadOutcome::Ok(ReadResult {
        kind: node.kind,
        path: format!("{file}/{}", node.name),
        start_line: Some(node.start_line),
        end_line: Some(node.end_line),
        source,
    }))
}

fn slice_file(path: &Path, start: i64, end: i64) -> Result<String> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    let start_idx = (start.max(1) as usize).saturating_sub(1);
    let end_idx = (end as usize).min(lines.len());
    Ok(lines[start_idx..end_idx].join("\n"))
}

const NODE_COLUMNS: &str = "id, name, kind, file_path, start_line, end_line";

fn node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NodeRow> {
    Ok(NodeRow {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        file_path: row.get(3)?,
        start_line: row.get(4)?,
        end_line: row.get(5)?,
    })
}

fn node_by_id(conn: &Connection, id: &str) -> Result<Option<NodeRow>> {
    if !id.contains(':') {
        return Ok(None);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM nodes WHERE id = ?1 AND kind != 'file'"
    ))?;
    let mut rows = stmt.query_map([id], node_from_row)?;
    Ok(rows.next().transpose()?)
}

fn nodes_in_file(conn: &Connection, file: &str, name: &str) -> Result<Vec<NodeRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM nodes
         WHERE file_path = ?1 AND name = ?2 AND kind != 'file'"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![file, name], node_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn resolve_symbols(conn: &Connection, symbol: &str) -> Result<Vec<NodeRow>> {
    if let Some(node) = node_by_id(conn, symbol)? {
        return Ok(vec![node]);
    }
    if let (Some(file), Some(name)) = virt_parts(&normalize_virt(Some(symbol))) {
        if file.contains('.') {
            let rows = nodes_in_file(conn, file, name)?;
            if !rows.is_empty() {
                return Ok(rows);
            }
        }
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM nodes WHERE name = ?1 AND kind != 'file'"
    ))?;
    let rows = stmt
        .query_map([symbol], node_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
