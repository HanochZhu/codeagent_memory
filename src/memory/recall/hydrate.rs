//! DB reads and writes for recall: candidate rows, breadcrumbs, retention.

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::ebbinghaus::strengthen;

/// One `solutions` row a fused candidate needs.
pub(super) struct SolutionRow {
    pub supersedes_id: Option<String>,
    pub summary: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub recalled_at: Option<i64>,
    pub stability: f64,
}

/// The full `solutions` row for `id`.
pub(super) fn load_solution(conn: &Connection, id: &str) -> Result<SolutionRow> {
    Ok(conn.query_row(
        "SELECT supersedes_id, summary, body, created_at, updated_at, recalled_at, stability
         FROM solutions WHERE id = ?1",
        [id],
        |row| {
            Ok(SolutionRow {
                supersedes_id: row.get(0)?,
                summary: row.get(1)?,
                body: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                recalled_at: row.get(5)?,
                stability: row.get(6)?,
            })
        },
    )?)
}

/// Refresh a directly matched memory's recall clock and stability.
pub(super) fn refresh_c0(conn: &Connection, id: &str, now: i64) -> Result<()> {
    let stability: f64 = conn.query_row(
        "SELECT stability FROM solutions WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE solutions SET recalled_at = ?1, stability = ?2 WHERE id = ?3",
        rusqlite::params![now, strengthen(stability), id],
    )?;
    Ok(())
}

/// Summary path from the root down to `id`.
pub(super) fn breadcrumb(conn: &Connection, id: &str) -> Result<Vec<String>> {
    let mut path = Vec::new();
    let mut current = Some(id.to_string());
    let mut guard = 0;
    while let Some(node) = current {
        guard += 1;
        if guard > 32 {
            break;
        }
        let (summary, parent): (String, Option<String>) = conn.query_row(
            "SELECT summary, parent_id FROM solutions WHERE id = ?1",
            [&node],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        path.push(summary);
        current = parent;
    }
    path.reverse();
    Ok(path)
}
