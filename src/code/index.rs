use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
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

pub fn index_project(project: &Project) -> Result<IndexReport> {
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;
    conn.execute_batch("DELETE FROM edges; DELETE FROM nodes; DELETE FROM files;")?;
    drop(conn);
    let sync = sync_project(project)?;
    Ok(IndexReport {
        files: sync.files_added,
        nodes: sync.nodes,
        edges: sync.edges,
        skipped: sync.skipped,
    })
}

pub fn sync_project(project: &Project) -> Result<SyncReport> {
    let started = Instant::now();
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;

    let (sources, mut skipped) = scan_source_files(project)?;
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
        return unchanged_sync_report(&conn, files_checked, skipped, started);
    }

    let mut extracted = Vec::new();
    let mut added_ok = 0usize;
    let mut modified_ok = 0usize;
    for src in &added {
        if let Some(file) = extract_or_skip(project, src, &mut skipped) {
            added_ok += 1;
            extracted.push(file);
        }
    }
    for src in &modified {
        if let Some(file) = extract_or_skip(project, src, &mut skipped) {
            modified_ok += 1;
            extracted.push(file);
        }
    }
    let changed_len = extracted.len();
    if changed_len == 0 && removed.is_empty() {
        return unchanged_sync_report(&conn, files_checked, skipped, started);
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
        for file in &extracted[..changed_len] {
            tx.execute("DELETE FROM nodes WHERE file_path = ?1", [&file.rel])?;
            insert_file_and_defs(&tx, &project.root, file)?;
        }
        tx.execute("DELETE FROM edges", [])?;
        tx.commit()?;
    }

    let name_index = load_name_index(&conn)?;
    {
        let tx = conn.unchecked_transaction()?;
        for file in &extracted {
            let Parsed { defs, rels, impls } = &file.parsed;
            insert_rels(&tx, &file.rel, defs, rels, &name_index)?;
            insert_impls(&tx, &file.rel, defs, impls, &name_index)?;
        }
        tx.commit()?;
    }

    Ok(SyncReport {
        files_checked,
        files_added: added_ok,
        files_modified: modified_ok,
        files_removed: removed.len(),
        nodes: db::table_count(&conn, "nodes")? as usize,
        edges: db::table_count(&conn, "edges")? as usize,
        skipped,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn scan_source_files(project: &Project) -> Result<(Vec<SourceFile>, usize)> {
    let mut skipped = 0usize;
    let mut sources = Vec::new();
    let walker = WalkBuilder::new(&project.root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        // The project root may be an umbrella folder that is not a git repo
        // itself; still honor .gitignore / .camignore found there.
        .require_git(false)
        .add_custom_ignore_filename(CAM_IGNORE_FILE)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            if skip_dir_name(&name) {
                return false;
            }
            // Nested git checkouts (submodules, vendored clones) are separate
            // projects; do not fold their symbols into this graph.
            entry.depth() == 0 || !is_linked_git_checkout(entry.path())
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
    }
    Ok((sources, skipped))
}

pub(crate) fn project_has_source_files(project: &Project) -> Result<bool> {
    let (sources, _) = scan_source_files(project)?;
    Ok(!sources.is_empty())
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

/// A directory whose `.git` is a file (gitlink) is a submodule or a linked
/// worktree: a second checkout of some other repository, not part of this one.
/// Standalone nested clones (`.git` directory) are still indexed, so an
/// umbrella folder holding several repos keeps working.
pub(crate) fn is_linked_git_checkout(dir: &Path) -> bool {
    dir.join(".git").is_file()
}

/// Cheap pre-filter used by the watcher: built-in skip dirs, the root
/// `.camignore`, and linked git checkouts between root and `path`.
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
    let mut cursor = root.to_path_buf();
    let mut parents = rel.components().peekable();
    while let Some(component) = parents.next() {
        if parents.peek().is_none() {
            break;
        }
        cursor.push(component);
        if is_linked_git_checkout(&cursor) {
            return true;
        }
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
