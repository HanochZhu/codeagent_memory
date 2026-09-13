use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;

use super::add::{escape_fts_query, tokenize_for_fts};
use super::ebbinghaus::{c0, needs_update, retention, strengthen};
use super::embed::{cosine, decode_f32, Embedder};
use crate::config::Config;
use crate::db;
use crate::project::Project;

const POOL: usize = 20;

#[derive(Debug, Clone)]
pub struct RawHit {
    pub id: String,
    pub vec_score: Option<f32>,
    pub bm25_score: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallHit {
    pub id: String,
    pub summary: String,
    pub body: String,
    pub score: f32,
    pub path: Vec<String>,
    pub stale: bool,
    pub needs_update: bool,
    pub retention: f64,
    pub latest: bool,
    pub age_days: i64,
    pub updated_at: i64,
}

pub fn fuse_scores(hits: &[RawHit]) -> Vec<(String, f32)> {
    let mut by_id: HashMap<String, (Option<f32>, Option<f32>)> = HashMap::new();
    for hit in hits {
        let entry = by_id.entry(hit.id.clone()).or_insert((None, None));
        if hit.vec_score.is_some() {
            entry.0 = hit.vec_score;
        }
        if hit.bm25_score.is_some() {
            entry.1 = hit.bm25_score;
        }
    }

    let vec_vals: Vec<f32> = by_id.values().filter_map(|v| v.0).collect();
    let bm25_vals: Vec<f32> = by_id.values().filter_map(|v| v.1).collect();

    let mut fused: Vec<(String, f32)> = by_id
        .into_iter()
        .map(|(id, (v, b))| {
            let p_vec = min_max(v, &vec_vals);
            let p_bm25 = min_max(b, &bm25_vals);
            (id, p_vec + p_bm25)
        })
        .collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

fn min_max(value: Option<f32>, all: &[f32]) -> f32 {
    let Some(value) = value else {
        return 0.0;
    };
    if all.is_empty() {
        return 0.0;
    }
    let min = all.iter().copied().fold(f32::INFINITY, f32::min);
    let max = all.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if (max - min).abs() < f32::EPSILON {
        return 1.0;
    }
    (value - min) / (max - min)
}

pub fn recall(
    project: &Project,
    embedder: &dyn Embedder,
    query: &str,
    limit: usize,
) -> Result<Vec<RecallHit>> {
    let conn = db::open_db(&project.db_path())?;
    let cfg = Config::load()?;
    let q_vec = embedder.embed(query)?;

    let mut vec_hits = Vec::new();
    {
        let mut stmt =
            conn.prepare("SELECT id, embedding FROM solutions WHERE embedding IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (id, blob) = row?;
            let emb = decode_f32(&blob);
            vec_hits.push((id, cosine(&q_vec, &emb)));
        }
    }
    vec_hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    vec_hits.truncate(POOL);

    let mut raw = Vec::new();
    for (id, score) in vec_hits {
        raw.push(RawHit {
            id,
            vec_score: Some(score),
            bm25_score: None,
        });
    }

    let fts = tokenize_for_fts(query, "");
    let match_q = escape_fts_query(&fts);
    if !match_q.is_empty() {
        let sql = format!(
            "SELECT s.id, bm25(solutions_fts) FROM solutions_fts
             JOIN solutions s ON s.rowid = solutions_fts.rowid
             WHERE solutions_fts MATCH ?1
             ORDER BY bm25(solutions_fts)
             LIMIT {POOL}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([&match_q], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        });
        if let Ok(rows) = rows {
            for row in rows {
                let (id, bm25) = row?;
                // FTS5 bm25 is lower (more negative) is better.
                raw.push(RawHit {
                    id,
                    vec_score: None,
                    bm25_score: Some(-bm25 as f32),
                });
            }
        }
    }

    let fused = fuse_scores(&raw);
    let now = chrono::Utc::now().timestamp();
    let mut hits = Vec::new();
    for (id, score) in fused {
        let (summary, body, parent_id, updated_at, recalled_at, stability): (
            String,
            String,
            Option<String>,
            i64,
            Option<i64>,
            f64,
        ) = conn.query_row(
            "SELECT summary, body, parent_id, updated_at, recalled_at, stability
             FROM solutions WHERE id = ?1",
            [&id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        let created_at: i64 = conn.query_row(
            "SELECT created_at FROM solutions WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )?;
        let r = retention(now, c0(recalled_at, created_at), stability);
        hits.push(RecallHit {
            id: id.clone(),
            summary,
            body,
            score: score + r as f32,
            path: breadcrumb(&conn, parent_id.as_deref(), &id)?,
            stale: cfg.is_stale(updated_at),
            needs_update: needs_update(r),
            retention: r,
            latest: true,
            age_days: Config::age_days(updated_at),
            updated_at,
        });
    }

    mark_latest_by_lineage(&mut hits);
    hits.sort_by(|a, b| {
        b.latest
            .cmp(&a.latest)
            .then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
    });
    hits.truncate(limit.max(1));

    for hit in &hits {
        if !hit.needs_update {
            refresh_c0(&conn, &hit.id, now)?;
        }
    }
    Ok(hits)
}

fn mark_latest_by_lineage(hits: &mut [RecallHit]) {
    let mut best: HashMap<String, (usize, i64)> = HashMap::new();
    for (i, hit) in hits.iter().enumerate() {
        let key = lineage_key(&hit.path);
        match best.get(&key) {
            Some((_, ts)) if hit.updated_at > *ts => {
                best.insert(key, (i, hit.updated_at));
            }
            None => {
                best.insert(key, (i, hit.updated_at));
            }
            _ => {}
        }
    }
    let winners: std::collections::HashSet<usize> = best.values().map(|(i, _)| *i).collect();
    for (i, hit) in hits.iter_mut().enumerate() {
        hit.latest = winners.contains(&i);
    }
}

fn lineage_key(path: &[String]) -> String {
    if path.len() <= 1 {
        return path.first().cloned().unwrap_or_default();
    }
    path[..path.len() - 1].join(" > ")
}

fn refresh_c0(conn: &rusqlite::Connection, id: &str, now: i64) -> Result<()> {
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

fn breadcrumb(
    conn: &rusqlite::Connection,
    parent_id: Option<&str>,
    id: &str,
) -> Result<Vec<String>> {
    let mut path = Vec::new();
    let mut current = parent_id.map(str::to_string);
    let mut guard = 0;
    while let Some(pid) = current {
        guard += 1;
        if guard > 32 {
            break;
        }
        let (summary, parent): (String, Option<String>) = conn.query_row(
            "SELECT summary, parent_id FROM solutions WHERE id = ?1",
            [&pid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        path.push(summary);
        current = parent;
    }
    path.reverse();
    let self_summary: String = conn.query_row(
        "SELECT summary FROM solutions WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    path.push(self_summary);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuse_sums_normalized_scores() {
        let hits = vec![
            RawHit {
                id: "a".into(),
                vec_score: Some(1.0),
                bm25_score: Some(10.0),
            },
            RawHit {
                id: "b".into(),
                vec_score: Some(0.0),
                bm25_score: Some(0.0),
            },
        ];
        let fused = fuse_scores(&hits);
        assert_eq!(fused[0].0, "a");
        assert!((fused[0].1 - 2.0).abs() < 1e-5);
        assert!((fused[1].1 - 0.0).abs() < 1e-5);
    }

    #[test]
    fn missing_path_is_zero() {
        let hits = vec![
            RawHit {
                id: "a".into(),
                vec_score: Some(1.0),
                bm25_score: None,
            },
            RawHit {
                id: "b".into(),
                vec_score: Some(0.0),
                bm25_score: Some(5.0),
            },
        ];
        let fused = fuse_scores(&hits);
        let map: HashMap<_, _> = fused.into_iter().collect();
        assert!((map["a"] - 1.0).abs() < 1e-5);
        assert!((map["b"] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn equal_scores_normalize_to_one() {
        let hits = vec![
            RawHit {
                id: "a".into(),
                vec_score: Some(0.4),
                bm25_score: None,
            },
            RawHit {
                id: "b".into(),
                vec_score: Some(0.4),
                bm25_score: None,
            },
        ];
        let fused = fuse_scores(&hits);
        assert!((fused[0].1 - 1.0).abs() < 1e-5);
        assert!((fused[1].1 - 1.0).abs() < 1e-5);
    }
}
