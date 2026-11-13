use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use ignore::gitignore::GitignoreBuilder;
use ignore::WalkBuilder;
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::lang::{parse_source, Def, Lang, Parsed, RelKind};
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

/// Bumped when the stored raw relation rows change shape. A database created
/// before `node_refs` existed re-parses every file once so later incremental
/// edge rebuilds have the rows they need.
const GRAPH_REFS_VERSION: &str = "1";

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
    /// Files tree-sitter parsed in this sync. Unchanged files are not parsed.
    pub files_parsed: usize,
    pub nodes: usize,
    pub edges: usize,
    pub skipped: usize,
    pub duration_ms: u64,
}

struct SourceFile {
    abs: PathBuf,
    rel: String,
    lang: Lang,
    size: i64,
    modified_at: i64,
    modified_ns: Option<i64>,
}

/// A file that will be (re)parsed. `hash` is already known when the caller had
/// to hash it to classify the change, so parsing does not hash twice.
struct PendingFile {
    src: SourceFile,
    hash: Option<String>,
}

struct FileStat {
    hash: String,
    size: i64,
    modified_ns: Option<i64>,
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
    with_graph_write(|| index_project_locked(project, on_progress))
}

/// Build the graph only when it is empty; otherwise apply the incremental
/// diff. This is what `cam index` and the MCP `cam_index` tool use by default,
/// so a reindex after editing a few files does not re-parse the whole project.
pub fn refresh_project(project: &Project) -> Result<IndexReport> {
    refresh_project_with_progress(project, &mut |_| {})
}

pub fn refresh_project_with_progress(
    project: &Project,
    on_progress: &mut dyn FnMut(IndexProgress),
) -> Result<IndexReport> {
    with_graph_write(|| {
        project.ensure_initialized()?;
        let conn = db::open_db(&project.db_path())?;
        let indexed = db::table_count(&conn, "files")? > 0;
        drop(conn);
        if indexed {
            let report = sync_tree(project)?;
            Ok(IndexReport {
                files: report.files_checked,
                nodes: report.nodes,
                edges: report.edges,
                skipped: report.skipped,
            })
        } else {
            index_project_locked(project, on_progress)
        }
    })
}

fn index_project_locked(
    project: &Project,
    on_progress: &mut dyn FnMut(IndexProgress),
) -> Result<IndexReport> {
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;
    conn.execute_batch(
        "DELETE FROM edges; DELETE FROM nodes; DELETE FROM files;
         DELETE FROM node_refs; DELETE FROM node_impls;",
    )?;
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
        let Some(child_report) =
            update_if_unlocked(&child, |child| index_project_locked(child, on_progress))
                .with_context(|| format!("index nested project {}", child.root.display()))?
        else {
            continue;
        };
        report.files += child_report.files;
        report.nodes += child_report.nodes;
        report.edges += child_report.edges;
        report.skipped += child_report.skipped;
    }
    Ok(report)
}

pub fn sync_project(project: &Project) -> Result<SyncReport> {
    with_graph_write(|| {
        let started = Instant::now();
        let mut report = sync_tree(project)?;
        report.duration_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    })
}

/// One writer at a time inside this process. The MCP thread may index while
/// the watcher thread syncs the same database.
fn with_graph_write<T>(body: impl FnOnce() -> Result<T>) -> Result<T> {
    thread_local! {
        static DEPTH: Cell<u32> = const { Cell::new(0) };
    }
    static GATE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    DEPTH.with(|depth| {
        if depth.get() > 0 {
            return body();
        }
        let _gate = GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        depth.set(1);
        struct Reset<'a>(&'a Cell<u32>);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.set(0);
            }
        }
        let _reset = Reset(depth);
        body()
    })
}

fn sync_tree(project: &Project) -> Result<SyncReport> {
    let mut own = sync_own(project, &mut |_| {})?;
    for rel in own.children {
        let child = Project {
            root: project.root.join(&rel),
        };
        let Some(child_report) = update_if_unlocked(&child, sync_tree)
            .with_context(|| format!("sync nested project {}", child.root.display()))?
        else {
            continue;
        };
        merge_sync(&mut own.report, child_report);
    }
    Ok(own.report)
}

/// Update a nested checkout only when its incremental lock is free.
///
/// A child MCP holds `.cam/incremental.lock` for as long as it is watching,
/// and the parent leaves that checkout's database untouched. With no holder,
/// the parent writes the child's `.cam/cam.db` and releases the lock.
fn update_if_unlocked<T>(
    child: &Project,
    update: impl FnOnce(&Project) -> Result<T>,
) -> Result<Option<T>> {
    super::lock::with_incremental_lock(child, update)
}

fn merge_sync(parent: &mut SyncReport, child: SyncReport) {
    parent.files_checked += child.files_checked;
    parent.files_added += child.files_added;
    parent.files_modified += child.files_modified;
    parent.files_removed += child.files_removed;
    parent.files_parsed += child.files_parsed;
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
    let existing = load_file_stats(&conn)?;
    let force_relations = graph_refs_version(&conn)?.as_deref() != Some(GRAPH_REFS_VERSION);
    let mut current = HashSet::new();
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut touched = Vec::new();
    let mut unchanged = Vec::new();

    for src in sources {
        current.insert(src.rel.clone());
        let Some(old) = existing.get(&src.rel) else {
            // New file: no hash needed before parsing, so read it once.
            added.push(PendingFile { src, hash: None });
            continue;
        };
        let same_stat =
            src.modified_ns.is_some() && old.size == src.size && old.modified_ns == src.modified_ns;
        if same_stat {
            unchanged.push(src);
            continue;
        }
        match hash_file(&src.abs) {
            Ok(hash) if hash == old.hash => touched.push(PendingFile {
                src,
                hash: Some(hash),
            }),
            Ok(hash) => modified.push(PendingFile {
                src,
                hash: Some(hash),
            }),
            Err(_) => {
                skipped += 1;
            }
        }
    }

    let removed: Vec<String> = existing
        .keys()
        .filter(|path| !current.contains(*path))
        .cloned()
        .collect();

    // A database indexed before raw relation rows existed cannot rebuild
    // dependents incrementally; re-parse every source once so they are stored.
    if force_relations {
        modified.extend(
            unchanged
                .drain(..)
                .map(|src| PendingFile { src, hash: None }),
        );
        modified.append(&mut touched);
    }

    let files_checked = added.len() + modified.len() + touched.len() + unchanged.len();
    if added.is_empty() && modified.is_empty() && removed.is_empty() {
        update_touched(&conn, &touched)?;
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
    for pending in &added {
        if let Some(file) = extract_or_skip(project, pending, &mut skipped) {
            added_ok += 1;
            extracted.push(file);
        }
        parsed += 1;
        report(progress, IndexPhase::Parsing, parsed, parse_total);
    }
    for pending in &modified {
        if let Some(file) = extract_or_skip(project, pending, &mut skipped) {
            modified_ok += 1;
            extracted.push(file);
        }
        parsed += 1;
        report(progress, IndexPhase::Parsing, parsed, parse_total);
    }
    let changed_len = extracted.len();
    if changed_len == 0 && removed.is_empty() {
        update_touched(&conn, &touched)?;
        return Ok(OwnSync {
            report: unchanged_sync_report(&conn, files_checked, skipped, started)?,
            children: child_rels,
        });
    }

    let changed_paths: HashSet<String> = extracted
        .iter()
        .map(|file| file.rel.clone())
        .chain(removed.iter().cloned())
        .collect();
    // When every file is being re-parsed, the affected set is already complete.
    let affected = affected_files(
        &conn,
        &changed_paths,
        &extracted,
        !force_relations && !existing.is_empty(),
    )?;

    if changed_len > 0 {
        report(progress, IndexPhase::Storing, 0, changed_len);
    }

    {
        let tx = conn.unchecked_transaction()?;
        for path in &removed {
            tx.execute("DELETE FROM nodes WHERE file_path = ?1", [path])?;
            tx.execute("DELETE FROM node_refs WHERE file_path = ?1", [path])?;
            tx.execute("DELETE FROM node_impls WHERE file_path = ?1", [path])?;
            tx.execute("DELETE FROM files WHERE path = ?1", [path])?;
        }
        for pending in &touched {
            tx.execute(
                "UPDATE files SET size = ?1, modified_at = ?2, modified_ns = ?3 WHERE path = ?4",
                rusqlite::params![
                    pending.src.size,
                    pending.src.modified_at,
                    pending.src.modified_ns,
                    &pending.src.rel
                ],
            )?;
        }
        for (i, file) in extracted.iter().enumerate() {
            tx.execute("DELETE FROM nodes WHERE file_path = ?1", [&file.rel])?;
            tx.execute("DELETE FROM node_refs WHERE file_path = ?1", [&file.rel])?;
            tx.execute("DELETE FROM node_impls WHERE file_path = ?1", [&file.rel])?;
            insert_file_and_defs(&tx, file)?;
            insert_relation_rows(&tx, file)?;
            insert_impl_rows(&tx, file)?;
            report(progress, IndexPhase::Storing, i + 1, changed_len);
        }

        let resolve_total = affected.len();
        if resolve_total > 0 {
            report(progress, IndexPhase::Resolving, 0, resolve_total);
        }
        let name_index = load_name_index(&tx)?;
        for (i, path) in affected.iter().enumerate() {
            delete_file_edges(&tx, path)?;
            rebuild_file_edges(&tx, path, &name_index)?;
            report(progress, IndexPhase::Resolving, i + 1, resolve_total);
        }
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('graph_refs_version', ?1)",
            [GRAPH_REFS_VERSION],
        )?;
        tx.commit()?;
    }

    Ok(OwnSync {
        report: SyncReport {
            files_checked,
            files_added: added_ok,
            files_modified: modified_ok,
            files_removed: removed.len(),
            files_parsed: parse_total,
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
        let meta = fs::metadata(path).ok();
        let size = meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        let (modified_at, modified_ns) = modified_parts(meta.as_ref());
        sources.push(SourceFile {
            abs: path.to_path_buf(),
            rel,
            lang,
            size,
            modified_at,
            modified_ns,
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

fn load_file_stats(conn: &rusqlite::Connection) -> Result<HashMap<String, FileStat>> {
    let mut stmt = conn.prepare("SELECT path, hash, size, modified_ns FROM files")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            FileStat {
                hash: row.get(1)?,
                size: row.get(2)?,
                modified_ns: row.get(3)?,
            },
        ))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (path, stat) = row?;
        map.insert(path, stat);
    }
    Ok(map)
}

fn update_touched(conn: &rusqlite::Connection, touched: &[PendingFile]) -> Result<()> {
    if touched.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for pending in touched {
        tx.execute(
            "UPDATE files SET size = ?1, modified_at = ?2, modified_ns = ?3 WHERE path = ?4",
            rusqlite::params![
                pending.src.size,
                pending.src.modified_at,
                pending.src.modified_ns,
                &pending.src.rel
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// Seconds and nanoseconds since the Unix epoch, from a file's mtime.
fn modified_parts(meta: Option<&fs::Metadata>) -> (i64, Option<i64>) {
    meta.and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|dur| {
            (
                dur.as_secs() as i64,
                Some(dur.as_nanos().min(i64::MAX as u128) as i64),
            )
        })
        .unwrap_or((0, None))
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
    hash: String,
    size: i64,
    modified_at: i64,
    modified_ns: Option<i64>,
}

struct StoredRef {
    source_id: String,
    target_name: String,
    edge_kind: String,
    line: i64,
    qualifier: Option<String>,
    prefer: String,
}

struct StoredImpl {
    type_name: String,
    trait_name: Option<String>,
    line: i64,
    method_id: Option<String>,
    method_line: Option<i64>,
}

fn extract_file(project: &Project, pending: &PendingFile) -> Result<ExtractedFile> {
    let src = &pending.src;
    let source =
        fs::read_to_string(&src.abs).with_context(|| format!("read {}", src.abs.display()))?;
    let parsed = parse_source(src.lang, &source)?;
    let hash = match &pending.hash {
        Some(hash) => hash.clone(),
        None => hex::encode(Sha256::digest(source.as_bytes())),
    };
    Ok(ExtractedFile {
        rel: rel_path(&project.root, &src.abs)?,
        lang: src.lang,
        parsed,
        hash,
        size: src.size,
        modified_at: src.modified_at,
        modified_ns: src.modified_ns,
    })
}

fn extract_or_skip(
    project: &Project,
    pending: &PendingFile,
    skipped: &mut usize,
) -> Option<ExtractedFile> {
    match extract_file(project, pending) {
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
        files_parsed: 0,
        nodes: db::table_count(conn, "nodes")? as usize,
        edges: db::table_count(conn, "edges")? as usize,
        skipped,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn insert_file_and_defs(conn: &Connection, file: &ExtractedFile) -> Result<()> {
    let rel = file.rel.as_str();
    let lang = file.lang;
    let defs = &file.parsed.defs;
    let now = chrono::Utc::now().timestamp();
    let modified_at = if file.modified_at > 0 {
        file.modified_at
    } else {
        now
    };
    conn.execute(
        "INSERT OR REPLACE INTO files(path, hash, language, size, modified_at, modified_ns, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            rel,
            &file.hash,
            lang.name(),
            file.size,
            modified_at,
            file.modified_ns,
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

/// Persist the unresolved call/reference sites of a file so its outgoing edges
/// can be rebuilt later without reading the source again.
fn insert_relation_rows(conn: &Connection, file: &ExtractedFile) -> Result<()> {
    let Parsed { defs, rels, .. } = &file.parsed;
    for site in rels {
        let Some(source_def) = innermost_def(defs, site.byte) else {
            continue;
        };
        let source_id = symbol_id(&file.rel, source_def);
        let (edge_kind, prefer) = match site.kind {
            RelKind::Calls => ("calls", "call"),
            RelKind::References => ("references", "type"),
        };
        conn.execute(
            "INSERT INTO node_refs(file_path, source_id, target_name, edge_kind, line, qualifier, prefer)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                &file.rel,
                source_id,
                &site.name,
                edge_kind,
                site.line,
                site.qualifier.as_deref(),
                prefer
            ],
        )?;
    }
    Ok(())
}

/// Persist impl blocks as a header row plus one row per method. Edges are
/// rebuilt from the stored names, so editing a type in another file updates
/// these without a re-parse.
fn insert_impl_rows(conn: &Connection, file: &ExtractedFile) -> Result<()> {
    let Parsed { defs, impls, .. } = &file.parsed;
    for block in impls {
        conn.execute(
            "INSERT INTO node_impls(file_path, type_name, trait_name, line, method_id, method_line)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            rusqlite::params![
                &file.rel,
                &block.type_name,
                block.trait_name.as_deref(),
                block.line
            ],
        )?;
        for def in defs {
            if def.kind != "method" {
                continue;
            }
            if def.start_byte < block.start_byte || def.start_byte > block.end_byte {
                continue;
            }
            let method_id = symbol_id(&file.rel, def);
            conn.execute(
                "INSERT INTO node_impls(file_path, type_name, trait_name, line, method_id, method_line)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    &file.rel,
                    &block.type_name,
                    block.trait_name.as_deref(),
                    block.line,
                    method_id,
                    def.start_line
                ],
            )?;
        }
    }
    Ok(())
}

/// Files whose outgoing edges must be rebuilt: the changed files themselves
/// plus unchanged files that reference a definition name touched by the
/// change. The latter fixes incoming edges when a target moved lines or was
/// renamed, without re-parsing those files.
fn affected_files(
    conn: &Connection,
    changed_paths: &HashSet<String>,
    extracted: &[ExtractedFile],
    expand_dependents: bool,
) -> Result<Vec<String>> {
    let mut affected: BTreeSet<String> = changed_paths.iter().cloned().collect();
    if !expand_dependents {
        return Ok(affected.into_iter().collect());
    }

    let mut names: BTreeSet<String> = BTreeSet::new();
    for path in changed_paths {
        let mut stmt =
            conn.prepare("SELECT name FROM nodes WHERE kind != 'file' AND file_path = ?1")?;
        let rows = stmt.query_map([path], |row| row.get::<_, String>(0))?;
        for row in rows {
            names.insert(row?);
        }
    }
    for file in extracted {
        for def in &file.parsed.defs {
            names.insert(def.name.clone());
        }
    }
    if names.is_empty() {
        return Ok(affected.into_iter().collect());
    }

    // Avoid SQLite's bound-parameter limit for a large change set. The inserts
    // share one transaction instead of committing once per name.
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "CREATE TEMP TABLE IF NOT EXISTS sync_names(name TEXT PRIMARY KEY)",
        [],
    )?;
    tx.execute("DELETE FROM sync_names", [])?;
    {
        let mut insert = tx.prepare("INSERT OR IGNORE INTO sync_names(name) VALUES (?1)")?;
        for name in &names {
            insert.execute([name])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT file_path FROM node_refs
                 JOIN sync_names ON node_refs.target_name = sync_names.name
             UNION
             SELECT DISTINCT file_path FROM node_refs
                 JOIN sync_names ON node_refs.qualifier = sync_names.name
             UNION
             SELECT DISTINCT file_path FROM node_impls
                 JOIN sync_names ON node_impls.type_name = sync_names.name
             UNION
             SELECT DISTINCT file_path FROM node_impls
                 JOIN sync_names ON node_impls.trait_name = sync_names.name",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            affected.insert(row?);
        }
    }
    tx.commit()?;
    Ok(affected.into_iter().collect())
}

fn delete_file_edges(conn: &Connection, path: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM edges WHERE source IN (SELECT id FROM nodes WHERE file_path = ?1)",
        [path],
    )?;
    Ok(())
}

fn rebuild_file_edges(
    conn: &Connection,
    path: &str,
    index: &HashMap<String, Vec<Candidate>>,
) -> Result<()> {
    let file_id = file_node_id(path);
    let defs: Vec<(String, i64)> = {
        let mut stmt = conn
            .prepare("SELECT id, start_line FROM nodes WHERE file_path = ?1 AND kind != 'file'")?;
        let rows = stmt.query_map([path], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (id, line) in &defs {
        insert_edge(conn, &file_id, id, "contains", *line)?;
    }

    let refs: Vec<StoredRef> = {
        let mut stmt = conn.prepare(
            "SELECT source_id, target_name, edge_kind, line, qualifier, prefer
             FROM node_refs WHERE file_path = ?1",
        )?;
        let rows = stmt.query_map([path], |row| {
            Ok(StoredRef {
                source_id: row.get(0)?,
                target_name: row.get(1)?,
                edge_kind: row.get(2)?,
                line: row.get(3)?,
                qualifier: row.get(4)?,
                prefer: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for reference in refs {
        let prefer_kinds = if reference.prefer == "call" {
            CALL_KINDS
        } else {
            TYPE_KINDS
        };
        for target_id in resolve_targets(
            index,
            path,
            &reference.target_name,
            prefer_kinds,
            reference.qualifier.as_deref(),
        ) {
            if reference.source_id == target_id {
                continue;
            }
            insert_edge(
                conn,
                &reference.source_id,
                &target_id,
                &reference.edge_kind,
                reference.line,
            )?;
        }
    }

    let impls: Vec<StoredImpl> = {
        let mut stmt = conn.prepare(
            "SELECT type_name, trait_name, line, method_id, method_line
             FROM node_impls WHERE file_path = ?1",
        )?;
        let rows = stmt.query_map([path], |row| {
            Ok(StoredImpl {
                type_name: row.get(0)?,
                trait_name: row.get(1)?,
                line: row.get(2)?,
                method_id: row.get(3)?,
                method_line: row.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for block in impls {
        let types = resolve_targets(index, path, &block.type_name, TYPE_KINDS, None);
        let traits = block
            .trait_name
            .as_deref()
            .map(|name| resolve_targets(index, path, name, TRAIT_KINDS, None))
            .unwrap_or_default();
        let Some(method_id) = block.method_id else {
            for type_id in &types {
                for trait_id in &traits {
                    if type_id == trait_id {
                        continue;
                    }
                    insert_edge(conn, type_id, trait_id, "implements", block.line)?;
                }
            }
            continue;
        };
        let method_line = block.method_line.unwrap_or(block.line);
        for type_id in &types {
            if method_id != *type_id {
                insert_edge(conn, type_id, &method_id, "contains", method_line)?;
            }
            insert_edge(conn, &method_id, type_id, "references", method_line)?;
            for trait_id in &traits {
                if method_id != *trait_id {
                    insert_edge(conn, &method_id, trait_id, "references", method_line)?;
                }
            }
        }
    }
    Ok(())
}

fn insert_edge(conn: &Connection, source: &str, target: &str, kind: &str, line: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![source, target, kind, line],
    )?;
    Ok(())
}

fn graph_refs_version(conn: &Connection) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'graph_refs_version'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
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
