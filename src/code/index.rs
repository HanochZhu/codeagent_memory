use std::fs;
use std::path::Path;

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
(trait_item name: (type_identifier) @name) @def
"#;

const RUST_CALLS: &str = r#"
(call_expression function: (identifier) @call)
(call_expression function: (field_expression field: (field_identifier) @call))
(call_expression function: (scoped_identifier name: (identifier) @call))
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

#[derive(Debug, Clone)]
struct CallSite {
    name: String,
    line: i64,
    byte: usize,
}

#[derive(Debug, Serialize)]
pub struct IndexReport {
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub skipped: usize,
}

pub fn index_project(project: &Project) -> Result<IndexReport> {
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;
    conn.execute_batch("DELETE FROM edges; DELETE FROM nodes; DELETE FROM files;")?;

    let mut files = 0usize;
    let mut skipped = 0usize;
    let mut extracted: Vec<(String, Lang, Vec<Def>, Vec<CallSite>)> = Vec::new();

    let walker = WalkBuilder::new(&project.root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "target" | "node_modules" | ".cam" | ".git" | "vendor" | "dist"
            )
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
        match extract_file(project, path, lang) {
            Ok((rel, defs, calls)) => {
                files += 1;
                extracted.push((rel, lang, defs, calls));
            }
            Err(_) => skipped += 1,
        }
    }

    {
        let tx = conn.unchecked_transaction()?;
        for (rel, lang, defs, _) in &extracted {
            insert_file_and_defs(&tx, &project.root, rel, *lang, defs)?;
        }
        tx.commit()?;
    }

    {
        let tx = conn.unchecked_transaction()?;
        for (rel, _, defs, calls) in &extracted {
            insert_calls(&tx, rel, defs, calls)?;
        }
        tx.commit()?;
    }

    Ok(IndexReport {
        files,
        nodes: db::table_count(&conn, "nodes")? as usize,
        edges: db::table_count(&conn, "edges")? as usize,
        skipped,
    })
}

fn extract_file(project: &Project, path: &Path, lang: Lang) -> Result<(String, Vec<Def>, Vec<CallSite>)> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let rel = rel_path(&project.root, path)?;
    let (defs, calls) = parse_source(lang, &source)?;
    Ok((rel, defs, calls))
}

fn parse_source(lang: Lang, source: &str) -> Result<(Vec<Def>, Vec<CallSite>)> {
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

    let call_query = Query::new(&language, lang.call_query())?;
    let mut call_cursor = QueryCursor::new();
    let mut calls = Vec::new();
    let mut call_matches = call_cursor.matches(&call_query, root, source.as_bytes());
    while let Some(m) = call_matches.next() {
        for cap in m.captures {
            if call_query.capture_names()[cap.index as usize] == "call" {
                let name = cap.node.utf8_text(source.as_bytes())?.to_string();
                if !name.is_empty() {
                    calls.push(CallSite {
                        name,
                        line: (cap.node.start_position().row + 1) as i64,
                        byte: cap.node.start_byte(),
                    });
                }
            }
        }
    }
    Ok((defs, calls))
}

fn classify_kind(lang: Lang, node: tree_sitter::Node, _name: &str, _source: &str) -> String {
    let kind = node.kind();
    match lang {
        Lang::Rust => match kind {
            "function_item" if has_ancestor(node, "impl_item") => "method",
            "function_item" => "function",
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

fn insert_calls(conn: &Connection, rel: &str, defs: &[Def], calls: &[CallSite]) -> Result<()> {
    for call in calls {
        let Some(source_def) = innermost_def(defs, call.byte) else {
            continue;
        };
        let source_id = symbol_id(rel, source_def);
        let Some(target_id) = resolve_call(conn, rel, &call.name)? else {
            continue;
        };
        if source_id == target_id {
            continue;
        }
        conn.execute(
            "INSERT OR IGNORE INTO edges(source, target, kind, line) VALUES (?1, ?2, 'calls', ?3)",
            rusqlite::params![source_id, target_id, call.line],
        )?;
    }
    Ok(())
}

fn innermost_def(defs: &[Def], byte: usize) -> Option<&Def> {
    defs.iter()
        .filter(|d| byte >= d.start_byte && byte <= d.end_byte)
        .min_by_key(|d| d.end_byte - d.start_byte)
}

fn resolve_call(conn: &Connection, file_path: &str, name: &str) -> Result<Option<String>> {
    let same_file: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM nodes WHERE file_path = ?1 AND name = ?2 AND kind != 'file'",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_path, name], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if same_file.len() == 1 {
        return Ok(same_file.into_iter().next());
    }

    let global: Vec<String> = {
        let mut stmt =
            conn.prepare("SELECT id FROM nodes WHERE name = ?1 AND kind != 'file'")?;
        let rows = stmt.query_map([name], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if global.len() == 1 {
        return Ok(global.into_iter().next());
    }
    Ok(None)
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
        let (defs, calls) = parse_source(Lang::Rust, src).unwrap();
        let names: Vec<_> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"run"));
        assert!(names.contains(&"helper"));
        let call_names: Vec<_> = calls.iter().map(|c| c.name.as_str()).collect();
        assert!(call_names.contains(&"add"));
        assert!(call_names.contains(&"helper"));
    }

    #[test]
    fn parse_python_functions_and_calls() {
        let src = "def helper():\n    return 1\ndef run():\n    return helper()\n";
        let (defs, calls) = parse_source(Lang::Python, src).unwrap();
        assert_eq!(defs.len(), 2);
        assert!(calls.iter().any(|c| c.name == "helper"));
    }
}
