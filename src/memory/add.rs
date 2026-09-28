use std::sync::OnceLock;

use anyhow::{bail, Result};
use jieba_rs::Jieba;
use serde::Serialize;
use uuid::Uuid;

use super::ebbinghaus::INITIAL_STABILITY_DAYS;
use super::embed::{encode_f32, Embedder};
use crate::db;
use crate::project::Project;

#[derive(Debug, Serialize)]
pub struct AddResult {
    pub id: String,
    pub parent_id: Option<String>,
    pub summary: String,
}

pub fn add_solution(
    project: &Project,
    embedder: &dyn Embedder,
    summary: &str,
    body: &str,
    parent_id: Option<&str>,
) -> Result<AddResult> {
    if summary.trim().is_empty() {
        bail!("summary is required");
    }
    if body.trim().is_empty() {
        bail!("body is required (stdin or --file)");
    }
    project.ensure_initialized()?;
    let conn = db::open_db(&project.db_path())?;
    if let Some(parent) = parent_id {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM solutions WHERE id = ?1",
            [parent],
            |row| row.get(0),
        )?;
        if exists == 0 {
            bail!("parent memory not found: {parent}");
        }
    }

    let id = Uuid::new_v4().simple().to_string()[..12].to_string();
    let now = chrono::Utc::now().timestamp();
    let fts_text = tokenize_for_fts(summary, body);
    let embedding = encode_f32(&embedder.embed(&format!("{summary}\n{body}"))?);

    conn.execute(
        "INSERT INTO solutions(id, parent_id, summary, body, created_at, updated_at, recalled_at, stability, embedding, fts_text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id,
            parent_id,
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
        parent_id: parent_id.map(str::to_string),
        summary: summary.trim().to_string(),
    })
}

fn jieba() -> &'static Jieba {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    JIEBA.get_or_init(Jieba::new)
}

fn cut(text: &str) -> impl Iterator<Item = &str> {
    jieba()
        .cut(text, false)
        .into_iter()
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Text stored for BM25. A camelCase identifier is kept whole and also split
/// into its words, so `bundledToolSchemas` matches both itself and a query
/// that spells it `bundled tool schemas`.
pub fn tokenize_for_fts(summary: &str, body: &str) -> String {
    let text = format!("{summary} {body}");
    let mut out = Vec::new();
    for token in cut(&text) {
        out.push(token.to_string());
        let words = camel_words(token);
        if words.len() > 1 {
            out.extend(words);
        }
    }
    out.join(" ")
}

/// Query terms for BM25. Function words are dropped: the query is an OR of
/// every term, so a word like 是否 or `the` only adds documents that share
/// nothing else with the question.
pub fn tokenize_query(query: &str) -> String {
    cut(query)
        .filter(|t| !is_stopword(t))
        .collect::<Vec<_>>()
        .join(" ")
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
    "的",
    "了",
    "是",
    "否",
    "是否",
    "由",
    "或",
    "或者",
    "和",
    "与",
    "及",
    "在",
    "吗",
    "呢",
    "吧",
    "啊",
    "把",
    "被",
    "就",
    "都",
    "也",
    "还",
    "又",
    "要",
    "会",
    "能",
    "可以",
    "这",
    "那",
    "这个",
    "那个",
    "这是",
    "一个",
    "有",
    "没有",
    "怎么",
    "怎样",
    "如何",
    "什么",
    "为什么",
    "哪",
    "哪里",
    "哪个",
    "a",
    "an",
    "the",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "do",
    "does",
    "did",
    "of",
    "to",
    "in",
    "on",
    "for",
    "by",
    "with",
    "or",
    "and",
    "it",
    "this",
    "that",
    "how",
    "what",
    "why",
    "which",
    "where",
    "when",
    "who",
    "whether",
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

    #[test]
    fn quotes_hyphen_and_or_tokens() {
        let q = escape_fts_query("multi-arch Docker OR");
        assert_eq!(q, "\"multi-arch\" OR \"Docker\" OR \"OR\"");
    }
}
