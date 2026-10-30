use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use ignore::gitignore::GitignoreBuilder;
use ignore::WalkBuilder;
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::lang::{parse_source, Def, ImplBlock, Lang, Parsed, Rel, RelKind};
use crate::db;
use crate::project::Project;

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    file_path: String,
    kind: String,
}

const CALL_KINDS: &[&str] = &["function", "method"];
const TYPE_KINDS: &[&str] = &["struct", "class", "trait", "enum", "type_alias"];
const TRAIT_KINDS: &[&str] = &["trait", "struct", "class"];
const MAX_CRATE_TIES: usize = 4;
/// gitignore-syntax file honored at any depth, in addition to `.gitignore`.
pub const CAM_IGNORE_FILE: &str = ".camignore";
pub(crate) const SKIP_DIR_NAMES: &[&str] =
    &["target", "node_modules", ".cam", ".git", "vendor", "dist"];

#[derive(Debug, Serialize)]
pub struct IndexReport {
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub skipped: usize,
}

/// Phase of a full index, in the order the indexer runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPhase {
    Scanning,
    Parsing,
    Storing,
    Resolving,
}

/// Progress event for [`index_project_with_progress`].
///
/// `total == 0` means the phase does not know its length yet (`current` is a
/// running count). Otherwise `current` advances toward `total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexProgress {
    pub phase: IndexPhase,
    pub current: usize,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub files_checked: usize,
    pub files_added: usize,
    pub files_modified: usize,
    pub files_removed: usize,
    pub nodes: usize,
    pub edges: usize,
    pub skipped: usize,
    pub duration_ms: u64,
}

struct SourceFile {
    abs: PathBuf,
    rel: String,
    lang: Lang,
}

/// A git checkout below the project being indexed. Its symbols live in that
/// directory's own `.cam/cam.db`; this project only stores `rel`.
#[derive(Clone)]
struct NestedProject {
    rel: String,
}

struct OwnSync {
    report: SyncReport,
    children: Vec<String>,
}

pub fn index_project(project: &Project) -> Result<IndexReport> {
    index_project_with_progress(project, &mut |_| {})
}

/// Full rebuild, invoking `on_progress` as each phase advances.
///
/// The callback must stay cheap: it runs on the indexing thread, including
/// inside the write transaction.
///
/// Nested directories with their own `.git` are rebuilt into that directory's
/// database. This project's database keeps those checkouts in `code_projects`
/// and stores symbols only for files whose nearest `.git` is this root.
pub fn index_project_with_progress(
    project: &Project,
    on_progress: &mut dyn FnMut(IndexProgress),
) -> Result<IndexReport> {
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;
    conn.execute_batch("DELETE FROM edges; DELETE FROM nodes; DELETE FROM files;")?;
    drop(conn);
    let own = sync_own(project, on_progress)?;
    let mut report = IndexReport {
        files: own.report.files_added,
        nodes: own.report.nodes,
        edges: own.report.edges,
        skipped: own.report.skipped,
    };
    for rel in &own.children {
        let child = Project {
            root: project.root.join(rel),
        };
        let child_report = index_project_with_progress(&child, on_progress)
            .with_context(|| format!("index nested project {}", child.root.display()))?;
        report.files += child_report.files;
        report.nodes += child_report.nodes;
        report.edges += child_report.edges;
        report.skipped += child_report.skipped;
    }
    Ok(report)
}

pub fn sync_project(project: &Project) -> Result<SyncReport> {
    let started = Instant::now();
    let mut report = sync_tree(project)?;
    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

fn sync_tree(project: &Project) -> Result<SyncReport> {
    let mut own = sync_own(project, &mut |_| {})?;
    for rel in own.children {
        let child = Project {
            root: project.root.join(&rel),
        };
        let child_report = sync_tree(&child)
            .with_context(|| format!("sync nested project {}", child.root.display()))?;
        merge_sync(&mut own.report, child_report);
    }
    Ok(own.report)
}

fn merge_sync(parent: &mut SyncReport, child: SyncReport) {
    parent.files_checked += child.files_checked;
    parent.files_added += child.files_added;
    parent.files_modified += child.files_modified;
    parent.files_removed += child.files_removed;
    parent.nodes += child.nodes;
    parent.edges += child.edges;
    parent.skipped += child.skipped;
}

fn sync_own(project: &Project, progress: &mut dyn FnMut(IndexProgress)) -> Result<OwnSync> {
    let started = Instant::now();
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;

    let (sources, children, mut skipped) = scan_source_files(project, progress)?;
    let child_rels: Vec<String> = children.into_iter().map(|child| child.rel).collect();
    write_code_projects(&conn, &child_rels)?;
    let existing = load_file_hashes(&conn)?;
    let mut current = HashSet::new();
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = Vec::new();

    for src in sources {
        current.insert(src.rel.clone());
        let hash = match hash_file(&src.abs) {
            Ok(h) => h,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        match existing.get(&src.rel) {
            None => added.push(src),
            Some(old) if old != &hash => modified.push(src),
            Some(_) => unchanged.push(src),
        }
    }

    let removed: Vec<String> = existing
        .keys()
        .filter(|path| !current.contains(*path))
        .cloned()
        .collect();

    let files_checked = added.len() + modified.len() + unchanged.len();
    if added.is_empty() && modified.is_empty() && removed.is_empty() {
        return Ok(OwnSync {
            report: unchanged_sync_report(&conn, files_checked, skipped, started)?,
            children: child_rels,
        });
    }

    let mut extracted = Vec::new();
    let mut added_ok = 0usize;
    let mut modified_ok = 0usize;
    let parse_total = added.len() + modified.len();
    if parse_total > 0 {
        report(progress, IndexPhase::Parsing, 0, parse_total);
    }
    let mut parsed = 0usize;
    for src in &added {
        if let Some(file) = extract_or_skip(project, src, &mut skipped) {
            added_ok += 1;
            extracted.push(file);
        }
        parsed += 1;
        report(progress, IndexPhase::Parsing, parsed, parse_total);
    }
    for src in &modified {
        if let Some(file) = extract_or_skip(project, src, &mut skipped) {
            modified_ok += 1;
            extracted.push(file);
        }
        parsed += 1;
        report(progress, IndexPhase::Parsing, parsed, parse_total);
    }
    let changed_len = extracted.len();
    if changed_len == 0 && removed.is_empty() {
        return Ok(OwnSync {
            report: unchanged_sync_report(&conn, files_checked, skipped, started)?,
            children: child_rels,
        });
    }
    for src in &unchanged {
        if let Some(file) = extract_or_skip(project, src, &mut skipped) {
            extracted.push(file);
        }
    }

    {
        let tx = conn.unchecked_transaction()?;
        for path in &removed {
            tx.execute("DELETE FROM nodes WHERE file_path = ?1", [path])?;
            tx.execute("DELETE FROM files WHERE path = ?1", [path])?;
        }
        if changed_len > 0 {
            report(progress, IndexPhase::Storing, 0, changed_len);
        }
        for (i, file) in extracted[..changed_len].iter().enumerate() {
            tx.execute("DELETE FROM nodes WHERE file_path = ?1", [&file.rel])?;
            insert_file_and_defs(&tx, &project.root, file)?;
            report(progress, IndexPhase::Storing, i + 1, changed_len);
        }
        tx.execute("DELETE FROM edges", [])?;
        tx.commit()?;
    }

    let resolve_total = extracted.len();
    if resolve_total > 0 {
        report(progress, IndexPhase::Resolving, 0, resolve_total);
    }
    let name_index = load_name_index(&conn)?;
    {
        let tx = conn.unchecked_transaction()?;
        for (i, file) in extracted.iter().enumerate() {
            let Parsed { defs, rels, impls } = &file.parsed;
            insert_rels(&tx, &file.rel, defs, rels, &name_index)?;
            insert_impls(&tx, &file.rel, defs, impls, &name_index)?;
            report(progress, IndexPhase::Resolving, i + 1, resolve_total);
        }
        tx.commit()?;
    }

    Ok(OwnSync {
        report: SyncReport {
            files_checked,
            files_added: added_ok,
            files_modified: modified_ok,
            files_removed: removed.len(),
            nodes: db::table_count(&conn, "nodes")? as usize,
            edges: db::table_count(&conn, "edges")? as usize,
            skipped,
            duration_ms: started.elapsed().as_millis() as u64,
        },
        children: child_rels,
    })
}

fn write_code_projects(conn: &Connection, rels: &[String]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM code_projects", [])?;
    for rel in rels {
        tx.execute("INSERT INTO code_projects(rel_path) VALUES (?1)", [rel])?;
    }
    tx.commit()?;
    Ok(())
}

fn report(
    progress: &mut dyn FnMut(IndexProgress),
    phase: IndexPhase,
    current: usize,
    total: usize,
) {
    progress(IndexProgress {
        phase,
        current,
        total,
    });
}

fn scan_source_files(
    project: &Project,
    progress: &mut dyn FnMut(IndexProgress),
) -> Result<(Vec<SourceFile>, Vec<NestedProject>, usize)> {
    report(progress, IndexPhase::Scanning, 0, 0);
    let mut skipped = 0usize;
    let mut sources = Vec::new();
    let mut found = 0usize;
    let nested = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&nested);
    let scan_root = project.root.clone();
    let walker = WalkBuilder::new(&project.root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        // The project root may be an umbrella folder that is not a git repo
        // itself; still honor .gitignore / .camignore found there.
        .require_git(false)
        .add_custom_ignore_filename(CAM_IGNORE_FILE)
        .filter_entry(move |entry| {
            let name = entry.file_name().to_string_lossy();
            if skip_dir_name(&name) {
                return false;
            }
            // A nested checkout keeps its own graph. Stop this walk at the
            // boundary so its symbols are not folded into the parent.
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            if entry.depth() > 0 && is_dir && is_git_project(entry.path()) {
                if let Ok(rel) = rel_path(&scan_root, entry.path()) {
                    if !rel.is_empty() {
                        record.lock().unwrap().push(NestedProject { rel });
                    }
                }
                return false;
            }
            true
        })
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Some(lang) = Lang::from_path(path) else {
            continue;
        };
        let rel = rel_path(&project.root, path)?;
        sources.push(SourceFile {
            abs: path.to_path_buf(),
            rel,
            lang,
        });
        found += 1;
        report(progress, IndexPhase::Scanning, found, 0);
    }
    let mut children = match Arc::try_unwrap(nested) {
        Ok(mutex) => mutex
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        Err(shared) => shared.lock().unwrap().clone(),
    };
    children.sort_by(|a, b| a.rel.cmp(&b.rel));
    children.dedup_by(|a, b| a.rel == b.rel);
    Ok((sources, children, skipped))
}

pub(crate) fn project_has_source_files(project: &Project) -> Result<bool> {
    let (sources, children, _) = scan_source_files(project, &mut |_| {})?;
    if !sources.is_empty() {
        return Ok(true);
    }
    for child in children {
        let nested = Project {
            root: project.root.join(child.rel),
        };
        if project_has_source_files(&nested)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_file_hashes(conn: &rusqlite::Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT path, hash FROM files")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut map = HashMap::new();
    for row in rows {
        let (path, hash) = row?;
        map.insert(path, hash);
    }
    Ok(map)
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

pub(crate) fn skip_dir_name(name: &str) -> bool {
    SKIP_DIR_NAMES.contains(&name)
}

/// A directory with its own `.git` entry is a separate checkout. A directory
/// is a clone; a file is a submodule or linked worktree (gitlink).
fn is_git_project(dir: &Path) -> bool {
    let git = dir.join(".git");
    git.is_dir() || git.is_file()
}

/// Cheap pre-filter used by the watcher: built-in skip dirs and the root
/// `.camignore`. Nested git checkouts stay visible so sync can update the
/// graph stored in that checkout.
/// `scan_source_files` remains the source of truth (it also honors nested
/// `.gitignore` / `.camignore` files), so a false negative here only costs a
/// no-op sync.
pub(crate) fn path_is_skipped(root: &Path, path: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return true,
    };
    if rel
        .components()
        .any(|c| c.as_os_str().to_str().is_some_and(skip_dir_name))
    {
        return true;
    }
    let ignore_file = root.join(CAM_IGNORE_FILE);
    if ignore_file.is_file() {
        let mut builder = GitignoreBuilder::new(root);
        if builder.add(&ignore_file).is_none() {
            if let Ok(rules) = builder.build() {
                let is_dir = path.is_dir();
                if rules.matched_path_or_any_parents(path, is_dir).is_ignore() {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn is_source_path(path: &Path) -> bool {
    Lang::from_path(path).is_some()
}

/// One source file after tree-sitter extraction, keyed by its project-relative path.
struct ExtractedFile {
    rel: String,
    lang: Lang,
    parsed: Parsed,
}

fn extract_file(project: &Project, path: &Path, lang: Lang) -> Result<ExtractedFile> {
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let rel = rel_path(&project.root, path)?;
    let parsed = parse_source(lang, &source)?;
    Ok(ExtractedFile { rel, lang, parsed })
}

fn extract_or_skip(
    project: &Project,
    src: &SourceFile,
    skipped: &mut usize,
) -> Option<ExtractedFile> {
    match extract_file(project, &src.abs, src.lang) {
        Ok(extracted) => Some(extracted),
        Err(_) => {
            *skipped += 1;
            None
        }
    }
}

fn unchanged_sync_report(
    conn: &Connection,
    files_checked: usize,
    skipped: usize,
    started: Instant,
) -> Result<SyncReport> {
    Ok(SyncReport {
        files_checked,
        files_added: 0,
        files_modified: 0,
        files_removed: 0,
        nodes: db::table_count(conn, "nodes")? as usize,
        edges: db::table_count(conn, "edges")? as usize,
        skipped,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn insert_file_and_defs(conn: &Connection, root: &Path, file: &ExtractedFile) -> Result<()> {
    let rel = file.rel.as_str();
    let lang = file.lang;
    let defs = &file.parsed.defs;
    let abs = root.join(rel);
    let meta = fs::metadata(&abs).ok();
    let bytes = fs::read(&abs).unwrap_or_default();
    let hash = hex::encode(Sha256::digest(&bytes));
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO files(path, hash, language, size, modified_at, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            rel,
            hash,
            lang.name(),
            meta.as_ref().map(|m| m.len() as i64).unwrap_or(0),
            meta.and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(now),
            now
        ],
    )?;

    let file_id = file_node_id(rel);
    conn.execute(
        "INSERT OR REPLACE INTO nodes(id, kind, name, file_path, start_line, end_line, signature)
         VALUES (?1, 'file', ?2, ?3, 1, 1, NULL)",
        rusqlite::params![file_id, file_name(rel), rel],
    )?;

    for def in defs {
        let id = symbol_id(rel, def);
        conn.execute(
            "INSERT OR REPLACE INTO nodes(id, kind, name, file_path, start_line, end_line, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id,
                def.kind,
                def.name,
                rel,
                def.start_line,
                def.end_line,
                def.signature
            ],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, 'contains', ?3)",
            rusqlite::params![file_id, id, def.start_line],
        )?;
    }
    Ok(())
}

fn insert_rels(
    conn: &Connection,
    rel: &str,
    defs: &[Def],
    rels: &[Rel],
    index: &HashMap<String, Vec<Candidate>>,
) -> Result<()> {
    for site in rels {
        let Some(source_def) = innermost_def(defs, site.byte) else {
            continue;
        };
        let source_id = symbol_id(rel, source_def);
        let (prefer, edge_kind) = match site.kind {
            RelKind::Calls => (CALL_KINDS, "calls"),
            RelKind::References => (TYPE_KINDS, "references"),
        };
        for target_id in resolve_targets(index, rel, &site.name, prefer, site.qualifier.as_deref())
        {
            if source_id == target_id {
                continue;
            }
            conn.execute(
                "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![source_id, target_id, edge_kind, site.line],
            )?;
        }
    }
    Ok(())
}

fn insert_impls(
    conn: &Connection,
    rel: &str,
    defs: &[Def],
    impls: &[ImplBlock],
    index: &HashMap<String, Vec<Candidate>>,
) -> Result<()> {
    for block in impls {
        let types = resolve_targets(index, rel, &block.type_name, TYPE_KINDS, None);
        let traits = block
            .trait_name
            .as_deref()
            .map(|name| resolve_targets(index, rel, name, TRAIT_KINDS, None))
            .unwrap_or_default();
        for type_id in &types {
            for trait_id in &traits {
                if type_id == trait_id {
                    continue;
                }
                conn.execute(
                    "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, 'implements', ?3)",
                    rusqlite::params![type_id, trait_id, block.line],
                )?;
            }
            for def in defs {
                if def.kind != "method" {
                    continue;
                }
                if def.start_byte < block.start_byte || def.start_byte > block.end_byte {
                    continue;
                }
                let method_id = symbol_id(rel, def);
                if method_id == *type_id {
                    continue;
                }
                conn.execute(
                    "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, 'contains', ?3)",
                    rusqlite::params![type_id, method_id, def.start_line],
                )?;
                conn.execute(
                    "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, 'references', ?3)",
                    rusqlite::params![method_id, type_id, def.start_line],
                )?;
                for trait_id in &traits {
                    if method_id == *trait_id {
                        continue;
                    }
                    conn.execute(
                        "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, 'references', ?3)",
                        rusqlite::params![method_id, trait_id, def.start_line],
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn innermost_def(defs: &[Def], byte: usize) -> Option<&Def> {
    defs.iter()
        .filter(|d| byte >= d.start_byte && byte <= d.end_byte)
        .min_by_key(|d| d.end_byte - d.start_byte)
}

fn load_name_index(conn: &Connection) -> Result<HashMap<String, Vec<Candidate>>> {
    let mut stmt =
        conn.prepare("SELECT id, name, kind, file_path FROM nodes WHERE kind != 'file'")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(1)?,
            Candidate {
                id: row.get(0)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
            },
        ))
    })?;
    let mut map: HashMap<String, Vec<Candidate>> = HashMap::new();
    for row in rows {
        let (name, cand) = row?;
        map.entry(name).or_default().push(cand);
    }
    Ok(map)
}

fn resolve_targets(
    index: &HashMap<String, Vec<Candidate>>,
    from_file: &str,
    name: &str,
    prefer: &[&str],
    qualifier: Option<&str>,
) -> Vec<String> {
    let Some(cands) = index.get(name) else {
        return Vec::new();
    };
    if cands.len() == 1 {
        return vec![cands[0].id.clone()];
    }
    let type_files = qualifier_type_files(index, qualifier);
    let mut scored: Vec<(i32, &Candidate)> = cands
        .iter()
        .map(|c| (resolve_score(from_file, name, c, prefer, &type_files), c))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let max = scored.first().map(|(s, _)| *s).unwrap_or(i32::MIN);
    if max < 0 {
        return Vec::new();
    }
    let best: Vec<&Candidate> = scored
        .iter()
        .filter(|(s, _)| *s == max)
        .map(|(_, c)| *c)
        .collect();
    if best.len() == 1 {
        return vec![best[0].id.clone()];
    }
    if best.iter().all(|c| c.file_path == from_file) {
        return best.into_iter().take(8).map(|c| c.id.clone()).collect();
    }
    let preferred: Vec<&Candidate> = best
        .iter()
        .copied()
        .filter(|c| prefer.iter().any(|k| c.kind == *k))
        .collect();
    if preferred.len() == 1 {
        return vec![preferred[0].id.clone()];
    }
    let pool = if preferred.is_empty() {
        best
    } else {
        preferred
    };
    keep_crate_ties(pool)
}

fn qualifier_type_files<'a>(
    index: &'a HashMap<String, Vec<Candidate>>,
    qualifier: Option<&str>,
) -> HashSet<&'a str> {
    let Some(name) = qualifier else {
        return HashSet::new();
    };
    index
        .get(name)
        .into_iter()
        .flatten()
        .filter(|c| TYPE_KINDS.iter().any(|k| c.kind == *k))
        .map(|c| c.file_path.as_str())
        .collect()
}

fn keep_crate_ties(pool: Vec<&Candidate>) -> Vec<String> {
    if pool.len() == 2 {
        return pool.into_iter().map(|c| c.id.clone()).collect();
    }
    if pool.is_empty() || pool.len() > MAX_CRATE_TIES {
        return Vec::new();
    }
    let crate0 = crate_prefix(&pool[0].file_path);
    if pool.iter().all(|c| crate_prefix(&c.file_path) == crate0) {
        pool.into_iter().map(|c| c.id.clone()).collect()
    } else {
        Vec::new()
    }
}

fn resolve_score(
    from: &str,
    name: &str,
    cand: &Candidate,
    prefer: &[&str],
    type_files: &HashSet<&str>,
) -> i32 {
    let mut score = 0;
    if cand.file_path == from {
        score += 100;
    } else if parent_dir(from) == parent_dir(&cand.file_path) {
        score += 50;
    } else if crate_prefix(from) == crate_prefix(&cand.file_path) {
        score += 20;
    }
    if prefer.iter().any(|k| cand.kind == *k) {
        score += 15;
    }
    if type_files.contains(cand.file_path.as_str()) {
        score += 40;
    }
    if prefer.iter().any(|k| TYPE_KINDS.contains(k))
        && file_stem(&cand.file_path).eq_ignore_ascii_case(name)
    {
        score += 25;
    }
    if is_test_path(&cand.file_path) && cand.file_path != from {
        score -= 40;
    }
    if cand.file_path.contains("examples/") && !from.contains("examples/") {
        score -= 20;
    }
    score
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

fn crate_prefix(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

fn file_stem(path: &str) -> &str {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
}

fn is_test_path(path: &str) -> bool {
    path.starts_with("tests/") || path.contains("/tests/") || path.contains("/test/")
}

fn file_node_id(rel: &str) -> String {
    format!("file:{rel}")
}

fn symbol_id(rel: &str, def: &Def) -> String {
    format!("{}:{}:{}:{}", rel, def.kind, def.name, def.start_line)
}

fn file_name(rel: &str) -> &str {
    Path::new(rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(rel)
}

fn rel_path(root: &Path, path: &Path) -> Result<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

pub fn virt_parts(path: &str) -> (Option<&str>, Option<&str>) {
    let path = path.trim_matches('/');
    if path.is_empty() {
        return (None, None);
    }
    if looks_like_source_file(path) {
        return (Some(path), None);
    }
    if let Some((file, symbol)) = split_file_symbol(path) {
        return (Some(file), Some(symbol));
    }
    (Some(path), None)
}

fn looks_like_source_file(path: &str) -> bool {
    Lang::from_path(Path::new(path)).is_some()
}

fn split_file_symbol(path: &str) -> Option<(&str, &str)> {
    let slash = path.rfind('/')?;
    let (prefix, name) = path.split_at(slash);
    let name = &name[1..];
    if looks_like_source_file(prefix) && !name.is_empty() {
        Some((prefix, name))
    } else {
        None
    }
}

pub fn normalize_virt(path: Option<&str>) -> String {
    path.unwrap_or("")
        .trim()
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

pub fn file_id(rel: &str) -> String {
    file_node_id(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_same_file_then_crate() {
        let mut idx = HashMap::new();
        idx.insert(
            "new".into(),
            vec![
                Candidate {
                    id: "a".into(),
                    file_path: "clap_builder/src/builder/command.rs".into(),
                    kind: "method".into(),
                },
                Candidate {
                    id: "b".into(),
                    file_path: "clap_builder/src/parser/parser.rs".into(),
                    kind: "method".into(),
                },
                Candidate {
                    id: "c".into(),
                    file_path: "tests/derive/foo.rs".into(),
                    kind: "method".into(),
                },
            ],
        );
        idx.insert(
            "Command".into(),
            vec![Candidate {
                id: "cmd".into(),
                file_path: "clap_builder/src/builder/command.rs".into(),
                kind: "struct".into(),
            }],
        );
        let same = resolve_targets(
            &idx,
            "clap_builder/src/builder/command.rs",
            "new",
            CALL_KINDS,
            None,
        );
        assert_eq!(same, vec!["a"]);
        let crate_hit = resolve_targets(
            &idx,
            "clap_builder/src/builder/mod.rs",
            "new",
            CALL_KINDS,
            None,
        );
        assert_eq!(crate_hit, vec!["a"]);
        let typed = resolve_targets(
            &idx,
            "tests/builder/help.rs",
            "new",
            CALL_KINDS,
            Some("Command"),
        );
        assert_eq!(typed, vec!["a"]);
    }

    #[test]
    fn resolve_keeps_small_same_crate_ties() {
        let mut idx = HashMap::new();
        idx.insert(
            "arg".into(),
            vec![
                Candidate {
                    id: "cmd_arg".into(),
                    file_path: "clap_builder/src/builder/command.rs".into(),
                    kind: "method".into(),
                },
                Candidate {
                    id: "group_arg".into(),
                    file_path: "clap_builder/src/builder/arg_group.rs".into(),
                    kind: "method".into(),
                },
            ],
        );
        let hits = resolve_targets(&idx, "tests/builder/help.rs", "arg", CALL_KINDS, None);
        assert_eq!(hits, vec!["cmd_arg", "group_arg"]);
    }

    #[test]
    fn resolve_drops_large_ambiguous_sets() {
        let mut idx = HashMap::new();
        idx.insert(
            "new".into(),
            (0..12)
                .map(|i| Candidate {
                    id: format!("n{i}"),
                    file_path: format!("clap_builder/src/foo{i}.rs"),
                    kind: "method".into(),
                })
                .collect(),
        );
        let hits = resolve_targets(&idx, "tests/builder/help.rs", "new", CALL_KINDS, None);
        assert!(hits.is_empty(), "{hits:?}");
    }
}
