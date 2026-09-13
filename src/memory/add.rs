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
            bail!("parent solution not found: {parent}");
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

pub fn tokenize_for_fts(summary: &str, body: &str) -> String {
    let jieba = Jieba::new();
    let text = format!("{summary} {body}");
    jieba
        .cut(&text, false)
        .into_iter()
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != " ")
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn escape_fts_query(tokens: &str) -> String {
    tokens
        .split_whitespace()
        .map(|t| {
            let cleaned: String = t
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            cleaned
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ")
}
