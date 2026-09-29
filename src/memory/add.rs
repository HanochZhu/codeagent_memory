use std::collections::HashSet;
use std::sync::OnceLock;

use anyhow::{bail, Result};
use rusqlite::OptionalExtension;
use rust_stemmers::{Algorithm, Stemmer};
use serde::Serialize;
use unicode_script::{Script, UnicodeScript};
use uuid::Uuid;

use super::ebbinghaus::INITIAL_STABILITY_DAYS;
use super::embed::{encode_f32, Embedder};
use crate::db;
use crate::project::Project;

#[derive(Debug, Serialize)]
pub struct AddResult {
    pub id: String,
    pub parent_id: Option<String>,
    pub supersedes_id: Option<String>,
    pub summary: String,
}

pub fn add_solution(
    project: &Project,
    embedder: &dyn Embedder,
    summary: &str,
    body: &str,
    parent_id: Option<&str>,
    supersedes_id: Option<&str>,
) -> Result<AddResult> {
    if summary.trim().is_empty() {
        bail!("summary is required");
    }
    if body.trim().is_empty() {
        bail!("body is required (stdin or --file)");
    }
    require_english("summary", summary)?;
    require_english("body", body)?;
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;
    let mut structural_parent = match parent_id {
        Some(parent) => {
            require_memory(&conn, "parent", parent)?;
            Some(parent.to_string())
        }
        None => None,
    };
    if let Some(superseded) = supersedes_id {
        let inherited_parent = require_memory(&conn, "superseded", superseded)?;
        let successor: Option<String> = conn
            .query_row(
                "SELECT id FROM solutions WHERE supersedes_id = ?1",
                [superseded],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(successor) = successor {
            bail!("memory {superseded} is already superseded by {successor}");
        }
        if structural_parent.is_none() {
            structural_parent = inherited_parent;
        }
    }

    let id = Uuid::new_v4().simple().to_string()[..12].to_string();
    let now = chrono::Utc::now().timestamp();
    let fts_text = tokenize_for_fts(summary, body);
    let embedding = encode_f32(&embedder.embed(&format!("{summary}\n{body}"))?);

    conn.execute(
        "INSERT INTO solutions(id, parent_id, supersedes_id, summary, body, created_at, updated_at, recalled_at, stability, embedding, fts_text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            id,
            structural_parent,
            supersedes_id,
            summary.trim(),
            body,
            now,
            now,
            now,
            INITIAL_STABILITY_DAYS,
            embedding,
            fts_text
        ],
    )?;

    Ok(AddResult {
        id,
        parent_id: structural_parent,
        supersedes_id: supersedes_id.map(str::to_string),
        summary: summary.trim().to_string(),
    })
}

fn require_memory(conn: &rusqlite::Connection, relation: &str, id: &str) -> Result<Option<String>> {
    match conn.query_row(
        "SELECT parent_id FROM solutions WHERE id = ?1",
        [id],
        |row| row.get(0),
    ) {
        Ok(parent) => Ok(parent),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            bail!("{relation} memory not found: {id}")
        }
        Err(err) => Err(err.into()),
    }
}

pub fn require_english(field: &str, text: &str) -> Result<()> {
    if let Some(ch) = text.chars().find(|ch| {
        ch.is_alphabetic()
            && !matches!(
                ch.script(),
                Script::Latin | Script::Common | Script::Inherited
            )
    }) {
        bail!(
            "{field} must be written in English; found non-Latin character `{ch}` (U+{:04X})",
            ch as u32
        );
    }
    Ok(())
}

fn stemmer() -> &'static Stemmer {
    static STEMMER: OnceLock<Stemmer> = OnceLock::new();
    STEMMER.get_or_init(|| Stemmer::create(Algorithm::English))
}

/// Text stored for BM25. A camelCase identifier is kept whole and also split
/// into its words, so `bundledToolSchemas` matches both itself and a query
/// that spells it `bundled tool schemas`. English words are stored in both
/// surface and stemmed forms, so inflections such as `stores` and `stored`
/// share a lexical term.
pub fn tokenize_for_fts(summary: &str, body: &str) -> String {
    let text = format!("{summary} {body}");
    english_terms(&text, false).join(" ")
}

/// Query terms for BM25. Function words are dropped: the query is an OR of
/// every term, so a word like `the` only adds documents that share nothing
/// else with the question.
pub fn tokenize_query(query: &str) -> String {
    english_terms(query, true).join(" ")
}

fn english_terms(text: &str, drop_stopwords: bool) -> Vec<String> {
    let mut terms = Vec::new();
    let mut query_seen = HashSet::new();
    for token in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if token.is_empty() {
            continue;
        }
        let mut variants = Vec::new();
        let mut variant_seen = HashSet::new();
        push_term(&mut variants, &mut variant_seen, token, drop_stopwords);
        for word in identifier_words(token) {
            push_term(&mut variants, &mut variant_seen, &word, drop_stopwords);
        }
        for term in variants {
            if !drop_stopwords || query_seen.insert(term.clone()) {
                terms.push(term);
            }
        }
    }
    terms
}

fn push_term(
    terms: &mut Vec<String>,
    seen: &mut HashSet<String>,
    term: &str,
    drop_stopwords: bool,
) {
    let normalized = term.to_lowercase();
    if normalized.is_empty() || (drop_stopwords && is_stopword(&normalized)) {
        return;
    }
    if seen.insert(normalized.clone()) {
        terms.push(normalized.clone());
    }
    if normalized.chars().all(|c| c.is_ascii_alphabetic()) {
        let stemmed = stemmer().stem(&normalized).into_owned();
        if seen.insert(stemmed.clone()) {
            terms.push(stemmed);
        }
    }
}

fn identifier_words(token: &str) -> Vec<String> {
    token.split(['_', '-']).flat_map(camel_words).collect()
}

fn camel_words(token: &str) -> Vec<String> {
    let chars: Vec<char> = token.chars().collect();
    if !chars.iter().all(|c| c.is_ascii_alphanumeric()) {
        return Vec::new();
    }
    let mut words = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let (prev, cur) = (chars[i - 1], chars[i]);
        let next_lower = chars.get(i + 1).is_some_and(|c| c.is_ascii_lowercase());
        let boundary = (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
            || (prev.is_ascii_uppercase() && cur.is_ascii_uppercase() && next_lower)
            || (prev.is_ascii_alphabetic() != cur.is_ascii_alphabetic());
        if boundary {
            words.push(chars[start..i].iter().collect());
            start = i;
        }
    }
    words.push(chars[start..].iter().collect());
    words
}

const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "do", "does", "did", "of", "to",
    "in", "on", "for", "by", "with", "or", "and", "it", "this", "that", "how", "what", "why",
    "which", "where", "when", "who", "whether",
];

fn is_stopword(token: &str) -> bool {
    STOPWORDS.iter().any(|s| s.eq_ignore_ascii_case(token))
}

pub fn escape_fts_query(tokens: &str) -> String {
    tokens
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::HashEmbedder;
    use tempfile::tempdir;

    #[test]
    fn rejects_non_latin_memory_text() {
        let err = require_english("query", "How does 向量 recall work?").unwrap_err();
        assert!(err.to_string().contains("must be written in English"));
    }

    #[test]
    fn allows_english_code_and_punctuation() {
        require_english(
            "body",
            "Use `bundledToolSchemas()`—it returns Vec<Result<T, E>>.",
        )
        .unwrap();
    }

    #[test]
    fn english_tokens_split_identifiers_and_add_stems() {
        let terms = tokenize_for_fts(
            "bundledToolSchemas",
            "Stores cached results and stores metadata",
        );
        assert!(terms.split_whitespace().any(|term| term == "bundled"));
        assert!(terms.split_whitespace().any(|term| term == "schemas"));
        assert!(terms.split_whitespace().any(|term| term == "store"));
        assert_eq!(
            terms
                .split_whitespace()
                .filter(|term| *term == "stores")
                .count(),
            2
        );
    }

    #[test]
    fn query_tokens_drop_stopwords_and_add_stems() {
        let terms = tokenize_query("How are stored memories recalled?");
        assert_eq!(terms, "stored store memories memori recalled recal");
    }

    #[test]
    fn quotes_hyphen_and_or_tokens() {
        let q = escape_fts_query("multi-arch Docker OR");
        assert_eq!(q, "\"multi-arch\" OR \"Docker\" OR \"OR\"");
    }

    #[test]
    fn revision_inherits_structural_parent_and_stays_linear() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let topic = add_solution(
            &project,
            &embedder,
            "Retry policy topic",
            "Retry policy topic body",
            None,
            None,
        )
        .unwrap();
        let old = add_solution(
            &project,
            &embedder,
            "Fixed retry policy",
            "Use fixed retry delays.",
            Some(&topic.id),
            None,
        )
        .unwrap();
        let revision = add_solution(
            &project,
            &embedder,
            "Jittered retry policy",
            "Use jittered retry delays.",
            None,
            Some(&old.id),
        )
        .unwrap();

        assert_eq!(revision.parent_id.as_deref(), Some(topic.id.as_str()));
        assert_eq!(revision.supersedes_id.as_deref(), Some(old.id.as_str()));

        let err = add_solution(
            &project,
            &embedder,
            "Alternative retry policy",
            "Use an alternative retry delay.",
            None,
            Some(&old.id),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already superseded"), "{err:#}");
    }
}
