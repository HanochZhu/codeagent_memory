//! The lexical path: FTS5 full-text match ranked by BM25.

use anyhow::Result;
use rusqlite::Connection;

use super::fuse::keep_near_top;
use crate::memory::add::{escape_fts_query, tokenize_query};

/// Share of the path's best score a memory needs to count as matched on this
/// path. RRF only sees ranks, so without a gate every document sharing one
/// near-zero-IDF token such as `ts` scores almost like a real match.
const BM25_KEEP: f32 = 0.3;

/// BM25-ranked FTS5 matches, gated to real matches.
pub(super) fn bm25_hits(conn: &Connection, query: &str, pool: usize) -> Result<Vec<(String, f32)>> {
    let mut hits = Vec::new();
    let match_q = escape_fts_query(&tokenize_query(query));
    if !match_q.is_empty() {
        let sql = format!(
            "SELECT s.id, bm25(solutions_fts) FROM solutions_fts
             JOIN solutions s ON s.rowid = solutions_fts.rowid
             WHERE solutions_fts MATCH ?1
             ORDER BY bm25(solutions_fts)
             LIMIT {pool}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([&match_q], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        });
        if let Ok(rows) = rows {
            for row in rows {
                let Ok((id, bm25)) = row else {
                    continue;
                };
                // FTS5 bm25 is lower (more negative) is better.
                hits.push((id, -bm25 as f32));
            }
        }
    }
    keep_near_top(&mut hits, BM25_KEEP);
    Ok(hits)
}
