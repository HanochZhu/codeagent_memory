use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::db;
use crate::project::Project;

const RUST_DEFS: &str = r#"
(function_item name: (identifier) @name) @def
(struct_item name: (type_identifier) @name) @def
(enum_item name: (type_identifier) @name) @def
(enum_variant name: (identifier) @name) @def
(trait_item name: (type_identifier) @name) @def
(type_item name: (type_identifier) @name) @def
"#;

const RUST_CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (field_expression field: (field_identifier) @call))
(call_expression function: (scoped_identifier name: (identifier) @call))
(call_expression function: (generic_function function: (identifier) @call))
(call_expression function: (generic_function function: (field_expression field: (field_identifier) @call)))
(call_expression function: (generic_function function: (scoped_identifier name: (identifier) @call)))
"#;

const RUST_REFS: &str = r#"
(type_identifier) @ref
(scoped_type_identifier name: (type_identifier) @ref)
(scoped_identifier path: (identifier) @ref)
(scoped_identifier path: (scoped_identifier name: (identifier) @ref))
"#;

const TS_REFS: &str = r#"
(type_identifier) @ref
"#;

const PYTHON_DEFS: &str = r#"
(function_definition name: (identifier) @name) @def
(class_definition name: (identifier) @name) @def
"#;

const PYTHON_CALLS: &str = r#"
(call function: (identifier) @call)
(call function: (attribute attribute: (identifier) @call))
"#;

const JS_DEFS: &str = r#"
(function_declaration name: (identifier) @name) @def
(generator_function_declaration name: (identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(class_declaration name: (identifier) @name) @def
"#;

const TS_DEFS: &str = r#"
(function_declaration name: (identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(class_declaration name: (type_identifier) @name) @def
"#;

const JS_CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (member_expression property: (property_identifier) @call))
"#;

const GO_DEFS: &str = r#"
(function_declaration name: (identifier) @name) @def
(method_declaration name: (field_identifier) @name) @def
(type_declaration (type_spec name: (type_identifier) @name type: (struct_type))) @def
"#;

const GO_CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (selector_expression field: (field_identifier) @call))
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
}

impl Lang {
    fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "js" | "mjs" | "cjs" | "jsx" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript | Self::Tsx => "typescript",
            Self::Go => "go",
        }
    }

    fn language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    fn def_query(self) -> &'static str {
        match self {
            Self::Rust => RUST_DEFS,
            Self::Python => PYTHON_DEFS,
            Self::JavaScript => JS_DEFS,
            Self::TypeScript | Self::Tsx => TS_DEFS,
            Self::Go => GO_DEFS,
        }
    }

    fn call_query(self) -> &'static str {
        match self {
            Self::Rust => RUST_CALLS,
            Self::Python => PYTHON_CALLS,
            Self::JavaScript | Self::TypeScript | Self::Tsx => JS_CALLS,
            Self::Go => GO_CALLS,
        }
    }

    fn ref_query(self) -> Option<&'static str> {
        match self {
            Self::Rust => Some(RUST_REFS),
            Self::TypeScript | Self::Tsx => Some(TS_REFS),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Def {
    kind: String,
    name: String,
    start_line: i64,
    end_line: i64,
    start_byte: usize,
    end_byte: usize,
    signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelKind {
    Calls,
    References,
}

#[derive(Debug, Clone)]
struct Rel {
    kind: RelKind,
    name: String,
    qualifier: Option<String>,
    line: i64,
    byte: usize,
}

#[derive(Debug, Clone)]
struct ImplBlock {
    type_name: String,
    trait_name: Option<String>,
    start_byte: usize,
    end_byte: usize,
    line: i64,
}

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    file_path: String,
    kind: String,
}

const SKIP_REF_NAMES: &[&str] = &[
    "Self", "self", "super", "crate", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
    "u32", "u64", "u128", "usize", "f32", "f64", "bool", "str", "char", "never",
];

const CALL_KINDS: &[&str] = &["function", "method"];
const TYPE_KINDS: &[&str] = &["struct", "class", "trait", "enum", "type_alias"];
const TRAIT_KINDS: &[&str] = &["trait", "struct", "class"];
const MAX_CRATE_TIES: usize = 4;
pub(crate) const SKIP_DIR_NAMES: &[&str] = &[
    "target",
    "node_modules",
    ".cam",
    ".git",
    "vendor",
    "dist",
];

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
        return Ok(SyncReport {
            files_checked,
            files_added: 0,
            files_modified: 0,
            files_removed: 0,
            nodes: db::table_count(&conn, "nodes")? as usize,
            edges: db::table_count(&conn, "edges")? as usize,
            skipped,
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }

    let added_paths: HashSet<String> = added.iter().map(|s| s.rel.clone()).collect();
    let mut added_ok = 0usize;
    let mut modified_ok = 0usize;
    let mut changed_extracted = Vec::new();
    let mut all_extracted = Vec::new();
    for src in added.iter().chain(modified.iter()) {
        match extract_file(project, &src.abs, src.lang) {
            Ok((rel, defs, rels, impls)) => {
                if added_paths.contains(&rel) {
                    added_ok += 1;
                } else {
                    modified_ok += 1;
                }
                let parsed = (rel, src.lang, defs, rels, impls);
                changed_extracted.push(parsed.clone());
                all_extracted.push(parsed);
            }
            Err(_) => skipped += 1,
        }
    }
    if changed_extracted.is_empty() && removed.is_empty() {
        return Ok(SyncReport {
            files_checked,
            files_added: 0,
            files_modified: 0,
            files_removed: 0,
            nodes: db::table_count(&conn, "nodes")? as usize,
            edges: db::table_count(&conn, "edges")? as usize,
            skipped,
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }
    for src in &unchanged {
        match extract_file(project, &src.abs, src.lang) {
            Ok((rel, defs, rels, impls)) => {
                all_extracted.push((rel, src.lang, defs, rels, impls));
            }
            Err(_) => skipped += 1,
        }
    }

    {
        let tx = conn.unchecked_transaction()?;
        for path in &removed {
            tx.execute("DELETE FROM nodes WHERE file_path = ?1", [path])?;
            tx.execute("DELETE FROM files WHERE path = ?1", [path])?;
        }
        for (rel, lang, defs, _, _) in &changed_extracted {
            tx.execute("DELETE FROM nodes WHERE file_path = ?1", [rel])?;
            insert_file_and_defs(&tx, &project.root, rel, *lang, defs)?;
        }
        tx.execute("DELETE FROM edges", [])?;
        tx.commit()?;
    }

    let name_index = load_name_index(&conn)?;
    {
        let tx = conn.unchecked_transaction()?;
        for (rel, _, defs, rels, impls) in &all_extracted {
            insert_rels(&tx, rel, defs, rels, &name_index)?;
            insert_impls(&tx, rel, defs, impls, &name_index)?;
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
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !skip_dir_name(&name)
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

pub(crate) fn path_is_skipped(root: &Path, path: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return true,
    };
    rel.components()
        .any(|c| c.as_os_str().to_str().is_some_and(skip_dir_name))
}

pub(crate) fn is_source_path(path: &Path) -> bool {
    Lang::from_path(path).is_some()
}

fn extract_file(
    project: &Project,
    path: &Path,
    lang: Lang,
) -> Result<(String, Vec<Def>, Vec<Rel>, Vec<ImplBlock>)> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let rel = rel_path(&project.root, path)?;
    let parsed = parse_source(lang, &source)?;
    Ok((rel, parsed.defs, parsed.rels, parsed.impls))
}

struct Parsed {
    defs: Vec<Def>,
    rels: Vec<Rel>,
    impls: Vec<ImplBlock>,
}

fn parse_source(lang: Lang, source: &str) -> Result<Parsed> {
    let language = lang.language();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("tree-sitter parse returned none")?;
    let root = tree.root_node();

    let def_query = Query::new(&language, lang.def_query())?;
    let mut cursor = QueryCursor::new();
    let mut defs = Vec::new();
    let mut matches = cursor.matches(&def_query, root, source.as_bytes());
    while let Some(m) = matches.next() {
        let mut def_node = None;
        let mut name = None;
        for cap in m.captures {
            let cap_name = def_query.capture_names()[cap.index as usize];
            match cap_name {
                "def" => def_node = Some(cap.node),
                "name" => name = Some(cap.node.utf8_text(source.as_bytes())?.to_string()),
                _ => {}
            }
        }
        let (Some(node), Some(name)) = (def_node, name) else {
            continue;
        };
        let kind = classify_kind(lang, node, &name, source);
        let first_line = source
            .get(node.start_byte()..node.end_byte())
            .and_then(|s| s.lines().next())
            .map(|s| s.trim().to_string());
        defs.push(Def {
            kind,
            name,
            start_line: (node.start_position().row + 1) as i64,
            end_line: (node.end_position().row + 1) as i64,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            signature: first_line,
        });
    }

    let mut rels = collect_named(&language, root, source, lang.call_query(), "call", RelKind::Calls)?;
    if let Some(q) = lang.ref_query() {
        rels.extend(collect_named(&language, root, source, q, "ref", RelKind::References)?);
        rels.retain(|r| {
            r.kind != RelKind::References || keep_ref_name(&r.name)
        });
    }

    let mut impls = Vec::new();
    walk_impls(lang, root, source, &mut impls);

    Ok(Parsed { defs, rels, impls })
}

fn collect_named(
    language: &Language,
    root: tree_sitter::Node,
    source: &str,
    query: &str,
    capture: &str,
    kind: RelKind,
) -> Result<Vec<Rel>> {
    let q = Query::new(language, query)?;
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut matches = cursor.matches(&q, root, source.as_bytes());
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if q.capture_names()[cap.index as usize] != capture {
                continue;
            }
            if kind == RelKind::References && is_type_def_name(cap.node) {
                continue;
            }
            let name = cap.node.utf8_text(source.as_bytes())?.to_string();
            if name.is_empty() {
                continue;
            }
            if kind == RelKind::References
                && cap.node.kind() == "identifier"
                && !looks_like_type_name(&name)
            {
                continue;
            }
            out.push(Rel {
                kind,
                name,
                qualifier: (kind == RelKind::Calls)
                    .then(|| call_qualifier(cap.node, source))
                    .flatten(),
                line: (cap.node.start_position().row + 1) as i64,
                byte: cap.node.start_byte(),
            });
        }
    }
    Ok(out)
}

fn is_type_def_name(node: tree_sitter::Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let def_item = matches!(
        parent.kind(),
        "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "enum_variant"
            | "class_declaration"
            | "class_definition"
            | "type_spec"
    );
    if !def_item {
        return false;
    }
    parent
        .child_by_field_name("name")
        .map(|n| n.id() == node.id())
        .unwrap_or(false)
}

fn keep_ref_name(name: &str) -> bool {
    if name.len() <= 1 {
        return false;
    }
    !SKIP_REF_NAMES.contains(&name)
}

fn looks_like_type_name(name: &str) -> bool {
    keep_ref_name(name) && name.chars().next().is_some_and(|c| c.is_uppercase())
}

fn call_qualifier(node: tree_sitter::Node, source: &str) -> Option<String> {
    let mut cur = node;
    if let Some(parent) = node.parent() {
        if parent.kind() == "generic_function" {
            cur = parent;
        }
    }
    let scoped = match cur.parent() {
        Some(p) if matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier") => p,
        Some(p) if p.kind() == "generic_function" => p
            .child_by_field_name("function")
            .filter(|f| matches!(f.kind(), "scoped_identifier" | "scoped_type_identifier"))?,
        _ => return None,
    };
    let path = scoped.child_by_field_name("path")?;
    trailing_type_name(path, source).filter(|s| looks_like_type_name(s))
}

fn walk_impls(lang: Lang, node: tree_sitter::Node, source: &str, out: &mut Vec<ImplBlock>) {
    match lang {
        Lang::Rust if node.kind() == "impl_item" => {
            let trait_name = node
                .child_by_field_name("trait")
                .and_then(|n| trailing_type_name(n, source));
            if let Some(type_name) = node
                .child_by_field_name("type")
                .and_then(|n| trailing_type_name(n, source))
            {
                if keep_ref_name(&type_name) {
                    out.push(ImplBlock {
                        type_name,
                        trait_name: trait_name.filter(|t| keep_ref_name(t)),
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                        line: (node.start_position().row + 1) as i64,
                    });
                }
            }
        }
        Lang::Python if node.kind() == "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(type_name) = name_node.utf8_text(source.as_bytes()) {
                    let type_name = type_name.to_string();
                    if let Some(supers) = node.child_by_field_name("superclasses") {
                        collect_ident_leaves(supers, source, &mut |base| {
                            if keep_ref_name(base) && base != type_name {
                                out.push(ImplBlock {
                                    type_name: type_name.clone(),
                                    trait_name: Some(base.to_string()),
                                    start_byte: node.start_byte(),
                                    end_byte: node.end_byte(),
                                    line: (node.start_position().row + 1) as i64,
                                });
                            }
                        });
                    }
                }
            }
        }
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx if node.kind() == "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(type_name) = name_node.utf8_text(source.as_bytes()) {
                    let type_name = type_name.to_string();
                    if let Some(base) = js_superclass(node, source) {
                        if keep_ref_name(&base) {
                            out.push(ImplBlock {
                                type_name,
                                trait_name: Some(base),
                                start_byte: node.start_byte(),
                                end_byte: node.end_byte(),
                                line: (node.start_position().row + 1) as i64,
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_impls(lang, child, source, out);
    }
}

fn js_superclass(node: tree_sitter::Node, source: &str) -> Option<String> {
    if let Some(heritage) = node.child_by_field_name("superclass") {
        return trailing_type_name(heritage, source);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_heritage" {
            return trailing_type_name(child, source);
        }
    }
    None
}

fn trailing_type_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" | "property_identifier" | "field_identifier" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|n| trailing_type_name(n, source)),
        "scoped_type_identifier" | "scoped_identifier" | "member_expression" => node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("property"))
            .and_then(|n| trailing_type_name(n, source))
            .or_else(|| {
                node.child(node.child_count().saturating_sub(1))
                    .and_then(|n| trailing_type_name(n, source))
            }),
        "reference_type" | "pointer_type" => node
            .child_by_field_name("type")
            .and_then(|n| trailing_type_name(n, source)),
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = trailing_type_name(child, source) {
                    return Some(name);
                }
            }
            None
        }
    }
}

fn collect_ident_leaves(node: tree_sitter::Node, source: &str, visit: &mut impl FnMut(&str)) {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        if let Ok(text) = node.utf8_text(source.as_bytes()) {
            visit(text);
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ident_leaves(child, source, visit);
    }
}

fn classify_kind(lang: Lang, node: tree_sitter::Node, _name: &str, _source: &str) -> String {
    let kind = node.kind();
    match lang {
        Lang::Rust => match kind {
            "function_item" if has_ancestor(node, "impl_item") => "method",
            "function_item" => "function",
            "trait_item" => "trait",
            "enum_item" => "enum",
            "type_item" => "type_alias",
            _ => "struct",
        }
        .into(),
        Lang::Python => match kind {
            "function_definition" if has_ancestor(node, "class_definition") => "method",
            "function_definition" => "function",
            _ => "class",
        }
        .into(),
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => match kind {
            "method_definition" => "method",
            "function_declaration" | "generator_function_declaration" => "function",
            _ => "class",
        }
        .into(),
        Lang::Go => match kind {
            "method_declaration" => "method",
            "function_declaration" => "function",
            _ => "struct",
        }
        .into(),
    }
}

fn has_ancestor(mut node: tree_sitter::Node, kind: &str) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return true;
        }
        node = parent;
    }
    false
}

fn insert_file_and_defs(
    conn: &Connection,
    root: &Path,
    rel: &str,
    lang: Lang,
    defs: &[Def],
) -> Result<()> {
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
        let prefer = match site.kind {
            RelKind::Calls => CALL_KINDS,
            RelKind::References => TYPE_KINDS,
        };
        let edge_kind = match site.kind {
            RelKind::Calls => "calls",
            RelKind::References => "references",
        };
        for target_id in resolve_targets(index, rel, &site.name, prefer, site.qualifier.as_deref()) {
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
    let pool = if preferred.is_empty() { best } else { preferred };
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
    fn parse_rust_functions_and_calls() {
        let src = r#"
pub fn add(a: i32, b: i32) -> i32 { a + b }
pub fn run() { let _ = add(1, 2); helper(); }
fn helper() {}
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        let names: Vec<_> = parsed.defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"run"));
        assert!(names.contains(&"helper"));
        let call_names: Vec<_> = parsed
            .rels
            .iter()
            .filter(|r| r.kind == RelKind::Calls)
            .map(|c| c.name.as_str())
            .collect();
        assert!(call_names.contains(&"add"));
        assert!(call_names.contains(&"helper"));
    }

    #[test]
    fn parse_rust_enum_variants_and_aliases() {
        let src = r#"
type Result<T> = std::result::Result<T, ()>;
enum Command { DoSomething { arg: String } }
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        let names: Vec<_> = parsed.defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Result"));
        assert!(names.contains(&"Command"));
        assert!(names.contains(&"DoSomething"));
        assert!(parsed.defs.iter().any(|d| d.name == "Command" && d.kind == "enum"));
        assert!(parsed.defs.iter().any(|d| d.name == "Result" && d.kind == "type_alias"));
    }

    #[test]
    fn parse_rust_impl_trait_and_type_refs() {
        let src = r#"
pub struct Foo;
pub trait Clone { fn clone(&self); }
impl Clone for Foo {
    fn clone(&self) {}
}
pub fn use_foo(x: Foo) { let _ = x.clone(); }
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        assert!(parsed.impls.iter().any(|i| i.type_name == "Foo" && i.trait_name.as_deref() == Some("Clone")));
        assert!(parsed.rels.iter().any(|r| r.kind == RelKind::References && r.name == "Foo"));
        assert!(parsed.rels.iter().any(|r| r.kind == RelKind::Calls && r.name == "clone"));
    }

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
        let hits = resolve_targets(
            &idx,
            "tests/builder/help.rs",
            "arg",
            CALL_KINDS,
            None,
        );
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
        let hits = resolve_targets(
            &idx,
            "tests/builder/help.rs",
            "new",
            CALL_KINDS,
            None,
        );
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn parse_rust_generic_and_qualified_calls() {
        let src = r#"
pub struct Foo;
impl Foo {
    fn new() -> Self { Foo }
    fn get_one<T>(&self) {}
}
pub fn run(x: Foo) {
    let _ = Foo::new();
    x.get_one::<u8>();
}
"#;
        let parsed = parse_source(Lang::Rust, src).unwrap();
        assert!(parsed.rels.iter().any(|r| r.kind == RelKind::Calls && r.name == "new" && r.qualifier.as_deref() == Some("Foo")));
        assert!(parsed.rels.iter().any(|r| r.kind == RelKind::Calls && r.name == "get_one"));
        assert!(parsed.rels.iter().any(|r| r.kind == RelKind::References && r.name == "Foo"));
    }

    #[test]
    fn parse_js_class_heritage() {
        let src = "class Foo extends Bar { method() { this.x(); } }\nclass Bar {}\n";
        let parsed = parse_source(Lang::JavaScript, src).unwrap();
        assert!(parsed.impls.iter().any(|i| i.type_name == "Foo" && i.trait_name.as_deref() == Some("Bar")));
    }

    #[test]
    fn parse_python_functions_and_calls() {
        let src = "def helper():\n    return 1\ndef run():\n    return helper()\n";
        let parsed = parse_source(Lang::Python, src).unwrap();
        assert_eq!(parsed.defs.len(), 2);
        assert!(parsed.rels.iter().any(|c| c.name == "helper"));
    }
}
