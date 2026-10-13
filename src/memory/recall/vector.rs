//! The vector path: cosine similarity between the query embedding and every
//! stored embedding.

use anyhow::Result;
use rusqlite::Connection;

use super::fuse::{keep_near_top, sort_desc};
use crate::memory::embed::{cosine, decode_f32, embedding_usable, Embedder};

/// Share of the path's best score a memory needs to count as matched on this
/// path. RRF only sees ranks, so without a gate every memory the vector path
/// returns scores almost like a real match.
const VEC_KEEP: f32 = 0.8;

/// Nearest memories by cosine similarity, gated to real matches.
pub(super) fn vec_hits(
    conn: &Connection,
    embedder: &dyn Embedder,
    q_vec: &[f32],
    pool: usize,
) -> Result<Vec<(String, f32)>> {
    let current_model = embedder.model_id();
    let mut hits = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, embedding, embedding_model FROM solutions WHERE embedding IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (id, blob, stored_model) = row?;
            let emb = decode_f32(&blob);
            if !embedding_usable(stored_model.as_deref(), &emb, current_model, q_vec) {
                continue;
            }
            hits.push((id, cosine(q_vec, &emb)));
        }
    }
    sort_desc(&mut hits);
    hits.truncate(pool);
    keep_near_top(&mut hits, VEC_KEEP);
    Ok(hits)
}
