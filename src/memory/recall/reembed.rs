//! Re-embedding stored vectors after an embedding-model change.

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::embed::{decode_f32, embedding_usable, encode_f32, Embedder};

/// Rewrite stored vectors that were produced by another model before this recall scores them.
/// Runs once per model change: later recalls see matching ids and leave the rows alone.
/// `updated_at` stays put so a vector refresh is not treated as an edit.
pub(super) fn reembed_mismatched(
    conn: &Connection,
    embedder: &dyn Embedder,
    query_vec: &[f32],
) -> Result<usize> {
    let current = embedder.model_id();
    let stale: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, summary, body, embedding, embedding_model FROM solutions WHERE embedding IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut stale = Vec::new();
        for row in rows {
            let (id, summary, body, blob, stored_model) = row?;
            let emb = decode_f32(&blob);
            if !embedding_usable(stored_model.as_deref(), &emb, current, query_vec) {
                stale.push((id, summary, body));
            }
        }
        stale
    };
    if stale.is_empty() {
        return Ok(0);
    }
    let tx = conn.unchecked_transaction()?;
    for (id, summary, body) in &stale {
        let embedding = encode_f32(&embedder.embed(&format!("{summary}\n{body}"))?);
        tx.execute(
            "UPDATE solutions SET embedding = ?1, embedding_model = ?2 WHERE id = ?3",
            rusqlite::params![embedding, current, id],
        )?;
    }
    tx.commit()?;
    let noun = if stale.len() == 1 {
        "memory"
    } else {
        "memories"
    };
    eprintln!("re-embedded {} {noun} with {current}", stale.len());
    Ok(stale.len())
}
