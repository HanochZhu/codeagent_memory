use std::collections::HashMap;
use std::fmt;

use anyhow::Result;
use clap::ValueEnum;
use serde::Serialize;

use super::add::{escape_fts_query, tokenize_for_fts};
use super::ebbinghaus::{c0, needs_update, retention, strengthen};
use super::embed::{cosine, decode_f32, Embedder};
use crate::config::Config;
use crate::project::Project;

const POOL: usize = 20;
/// Cormack et al. RRF constant. Raw RRF is scaled by `(k + 1)` so a rank-1
/// hit on one list scores 1.0 and a rank-1 hit on both lists scores 2.0 —
/// the same range as min-max sum fusion — before Ebbinghaus retention is added.
const RRF_K: f32 = 60.0;
/// Fraction of the fused score a chain tail inherits from the seed that pulled
/// it in, so structure alone does not look like a strong topical match. It only
/// discounts the fused part; Ebbinghaus retention is added on top afterwards.
const EXPAND_DECAY: f32 = 0.5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fusion {
    /// Reciprocal Rank Fusion over the vector and BM25 ranked lists.
    #[default]
    Rrf,
    /// Min-max each path to [0, 1] then sum.
    Sum,
}

impl fmt::Display for Fusion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fusion::Sum => write!(f, "sum"),
            Fusion::Rrf => write!(f, "rrf"),
        }
    }
}

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

fn merge_hits(hits: &[RawHit]) -> HashMap<String, (Option<f32>, Option<f32>)> {
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
    by_id
}

pub fn fuse(hits: &[RawHit], fusion: Fusion) -> Vec<(String, f32)> {
    match fusion {
        Fusion::Sum => fuse_scores(hits),
        Fusion::Rrf => fuse_rrf(hits, RRF_K),
    }
}

/// Min-max each ranked list to `[0, 1]` and sum. A document missing from a list
/// contributes 0 for that list.
pub fn fuse_scores(hits: &[RawHit]) -> Vec<(String, f32)> {
    let by_id = merge_hits(hits);
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
    sort_desc(&mut fused);
    fused
}

/// Reciprocal Rank Fusion: `score(d) = (k + 1) * Σ 1/(k + rank_i(d))`.
/// Rank is 1-based. A document missing from a list contributes 0 for that list.
pub fn fuse_rrf(hits: &[RawHit], k: f32) -> Vec<(String, f32)> {
    let by_id = merge_hits(hits);

    let mut vec_ranked: Vec<(String, f32)> = by_id
        .iter()
        .filter_map(|(id, (v, _))| v.map(|s| (id.clone(), s)))
        .collect();
    sort_desc(&mut vec_ranked);

    let mut bm25_ranked: Vec<(String, f32)> = by_id
        .iter()
        .filter_map(|(id, (_, b))| b.map(|s| (id.clone(), s)))
        .collect();
    sort_desc(&mut bm25_ranked);

    let mut scores: HashMap<String, f32> = HashMap::new();
    add_rrf_ranks(&mut scores, &vec_ranked, k);
    add_rrf_ranks(&mut scores, &bm25_ranked, k);

    let scale = k + 1.0;
    let mut fused: Vec<(String, f32)> = scores
        .into_iter()
        .map(|(id, s)| (id, s * scale))
        .collect();
    sort_desc(&mut fused);
    fused
}

fn add_rrf_ranks(scores: &mut HashMap<String, f32>, ranked: &[(String, f32)], k: f32) {
    for (rank, (id, _)) in ranked.iter().enumerate() {
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + rank as f32 + 1.0);
    }
}

fn sort_desc(items: &mut [(String, f32)]) {
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
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

/// Knobs for one `recall` call.
#[derive(Debug, Clone, Copy)]
pub struct RecallOptions {
    pub limit: usize,
    pub fusion: Fusion,
    /// Also pull in the newest revision of whatever the query matched, even
    /// when that revision matches neither the vector nor the BM25 path.
    pub expand: bool,
}

impl Default for RecallOptions {
    fn default() -> Self {
        Self {
            limit: 3,
            fusion: Fusion::Rrf,
            expand: true,
        }
    }
}

pub fn recall(
    project: &Project,
    embedder: &dyn Embedder,
    query: &str,
    opts: RecallOptions,
) -> Result<Vec<RecallHit>> {
    let conn = project.connect()?;
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
                let Ok((id, bm25)) = row else {
                    continue;
                };
                // FTS5 bm25 is lower (more negative) is better.
                raw.push(RawHit {
                    id,
                    vec_score: None,
                    bm25_score: Some(-bm25 as f32),
                });
            }
        }
    }

    let fused = fuse(&raw, opts.fusion);
    let seeds = if opts.expand { opts.limit.max(1) } else { 0 };
    let mut chains = ChainIndex::default();
    let expanded = chain_tails(&conn, &mut chains, &fused, seeds)?;
    // Structure is not a recall. A tail that only got here through its chain
    // must not refresh retention, or a hot memory would keep its whole lineage
    // permanently fresh.
    let pulled_in: std::collections::HashSet<String> =
        expanded.iter().map(|(id, _)| id.clone()).collect();
    let candidates: Vec<(String, f32)> = fused.into_iter().chain(expanded).collect();
    let now = chrono::Utc::now().timestamp();
    let mut hits = Vec::new();
    for (id, score) in candidates {
        let (summary, body, created_at, updated_at, recalled_at, stability): (
            String,
            String,
            i64,
            i64,
            Option<i64>,
            f64,
        ) = conn.query_row(
            "SELECT summary, body, created_at, updated_at, recalled_at, stability
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
        let r = retention(now, c0(recalled_at, created_at), stability);
        let latest = chains.tail_of(&conn, &id)? == id;
        hits.push(RecallHit {
            id,
            summary,
            body,
            score: score + r as f32,
            path: Vec::new(),
            stale: cfg.is_stale(updated_at),
            needs_update: needs_update(r),
            retention: r,
            latest,
            age_days: Config::age_days(updated_at),
            updated_at,
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(opts.limit.max(1));

    for hit in &mut hits {
        hit.path = breadcrumb(&conn, &hit.id)?;
    }
    for hit in &hits {
        if !hit.needs_update && !pulled_in.contains(&hit.id) {
            refresh_c0(&conn, &hit.id, now)?;
        }
    }
    Ok(hits)
}

/// Chain tails worth adding to the fused list, scored off the top `seeds`.
///
/// A revision is stored as a new child rather than an edit, so the newest node
/// on a chain often shares no wording with the query and neither the vector nor
/// the BM25 path can reach it. Pulling the tail in keeps the current answer
/// reachable even when the query only matches a revision several steps behind.
fn chain_tails(
    conn: &rusqlite::Connection,
    chains: &mut ChainIndex,
    fused: &[(String, f32)],
    seeds: usize,
) -> Result<Vec<(String, f32)>> {
    let matched: std::collections::HashSet<&str> =
        fused.iter().map(|(id, _)| id.as_str()).collect();
    let mut added: HashMap<String, f32> = HashMap::new();

    for (id, score) in fused.iter().take(seeds) {
        let tail = chains.tail_of(conn, id)?;
        if tail == *id || matched.contains(tail.as_str()) {
            continue;
        }
        let decayed = score * EXPAND_DECAY;
        let best = added.entry(tail).or_insert(decayed);
        *best = best.max(decayed);
    }

    let mut tails: Vec<(String, f32)> = added.into_iter().collect();
    sort_desc(&mut tails);
    Ok(tails)
}

/// Memoised chain lookups. Resolving one member caches the tail for the whole
/// chain, so a recall that touches several revisions of the same memory still
/// costs one query.
#[derive(Default)]
struct ChainIndex {
    tails: HashMap<String, String>,
}

impl ChainIndex {
    fn tail_of(&mut self, conn: &rusqlite::Connection, id: &str) -> Result<String> {
        if let Some(tail) = self.tails.get(id) {
            return Ok(tail.clone());
        }
        let chain = lineage_chain(conn, id)?;
        let tail = chain.last().cloned().unwrap_or_else(|| id.to_string());
        for member in chain {
            self.tails.insert(member, tail.clone());
        }
        self.tails.insert(id.to_string(), tail.clone());
        Ok(tail)
    }
}

/// Every memory on the same revision chain as `id`, oldest first, so the last
/// entry is the tail.
///
/// Walks up to the root and back down again: a seed in the middle of a chain
/// resolves to the newest revision, not just to its immediate neighbours.
/// `updated_at` only has second resolution, so `rowid` breaks ties by insert
/// order rather than leaving the tail up to retrieval order. `UNION`
/// de-duplicates, so a `parent_id` cycle terminates instead of looping; such a
/// chain has no root, yields no rows, and the caller falls back to `id`.
fn lineage_chain(conn: &rusqlite::Connection, id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "WITH RECURSIVE
             ancestors(id, parent_id) AS (
                 SELECT id, parent_id FROM solutions WHERE id = ?1
                 UNION
                 SELECT s.id, s.parent_id FROM solutions s
                 JOIN ancestors a ON s.id = a.parent_id
             ),
             chain(id) AS (
                 SELECT id FROM ancestors WHERE parent_id IS NULL
                 UNION
                 SELECT s.id FROM solutions s JOIN chain c ON s.parent_id = c.id
             )
         SELECT c.id FROM chain c
         JOIN solutions s ON s.id = c.id
         ORDER BY s.updated_at, s.rowid",
    )?;
    let chain = stmt
        .query_map([id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(chain)
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

fn breadcrumb(conn: &rusqlite::Connection, id: &str) -> Result<Vec<String>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{add_solution, HashEmbedder};
    use tempfile::tempdir;

    fn add(project: &Project, summary: &str, body: &str, parent: Option<&str>) -> String {
        add_solution(project, &HashEmbedder::default(), summary, body, parent)
            .unwrap()
            .id
    }

    #[test]
    fn chain_tails_reaches_a_tail_two_hops_away() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let root = add(&project, "root", "root body", None);
        let seed = add(&project, "seed", "seed body", Some(&root));
        let tail = add(&project, "tail", "tail body", Some(&seed));

        let conn = project.connect().unwrap();
        let mut chains = ChainIndex::default();
        let tails = chain_tails(&conn, &mut chains, &[(seed, 2.0)], 1).unwrap();

        assert_eq!(tails.len(), 1, "a superseded ancestor stays out: {tails:?}");
        assert_eq!(tails[0].0, tail);
        assert!((tails[0].1 - 1.0).abs() < 1e-5);
        assert_ne!(tails[0].0, root);
    }

    #[test]
    fn chain_tails_skips_a_tail_the_query_already_matched() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let root = add(&project, "root", "root body", None);
        let tail = add(&project, "tail", "tail body", Some(&root));

        let conn = project.connect().unwrap();
        let mut chains = ChainIndex::default();
        let tails = chain_tails(&conn, &mut chains, &[(root, 2.0), (tail, 0.2)], 2).unwrap();

        assert!(tails.is_empty(), "{tails:?}");
    }

    #[test]
    fn zero_seeds_skips_expansion() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let root = add(&project, "root", "root body", None);
        add(&project, "tail", "tail body", Some(&root));

        let conn = project.connect().unwrap();
        let mut chains = ChainIndex::default();
        let tails = chain_tails(&conn, &mut chains, &[(root, 2.0)], 0).unwrap();

        assert!(tails.is_empty(), "{tails:?}");
    }

    #[test]
    fn latest_marks_only_the_chain_tail() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let root = add(
            &project,
            "retry policy uses fixed backoff",
            "retry policy uses fixed backoff",
            None,
        );
        let mid = add(
            &project,
            "retry policy rev1 exponential backoff",
            "retry policy rev1 exponential backoff",
            Some(&root),
        );
        let tail = add(
            &project,
            "retry policy rev2 jittered backoff",
            "retry policy rev2 jittered backoff",
            Some(&mid),
        );

        let hits = recall(
            &project,
            &embedder,
            "retry policy backoff",
            RecallOptions {
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&root.as_str()), "{ids:?}");
        assert!(ids.contains(&mid.as_str()), "{ids:?}");

        let latest: Vec<&str> = hits
            .iter()
            .filter(|h| h.latest)
            .map(|h| h.id.as_str())
            .collect();
        assert_eq!(latest, vec![tail.as_str()], "{hits:?}");
    }

    #[test]
    fn recall_surfaces_a_revision_neither_path_can_reach() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let old = add(
            &project,
            "multi path recall fuses bm25 and vectors",
            "min-max each path then sum",
            None,
        );
        let revision = add(&project, "zzz", "zzz", Some(&old));

        let conn = project.connect().unwrap();
        conn.execute(
            "UPDATE solutions SET embedding = NULL, fts_text = '', updated_at = updated_at + 60
             WHERE id = ?1",
            [&revision],
        )
        .unwrap();
        drop(conn);

        let hits = recall(
            &project,
            &embedder,
            "bm25 and vectors",
            RecallOptions::default(),
        )
        .unwrap();
        let pulled_in = hits
            .iter()
            .find(|h| h.id == revision)
            .expect("revision reached only through its parent");
        assert!(pulled_in.latest);
        assert!(!hits.iter().find(|h| h.id == old).unwrap().latest);
    }

    #[test]
    fn a_structure_pulled_tail_does_not_refresh_retention() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let matched = add(
            &project,
            "multi path recall fuses bm25 and vectors",
            "min-max each path then sum",
            None,
        );
        let revision = add(&project, "zzz", "zzz", Some(&matched));

        let conn = project.connect().unwrap();
        conn.execute(
            "UPDATE solutions SET embedding = NULL, fts_text = '' WHERE id = ?1",
            [&revision],
        )
        .unwrap();
        let before = stability_of(&conn, &revision);
        drop(conn);

        let hits = recall(
            &project,
            &embedder,
            "bm25 and vectors",
            RecallOptions::default(),
        )
        .unwrap();
        assert!(hits.iter().any(|h| h.id == revision), "{hits:?}");

        let conn = project.connect().unwrap();
        assert!(
            (stability_of(&conn, &revision) - before).abs() < 1e-9,
            "structure alone must not strengthen a memory"
        );
        assert!(
            stability_of(&conn, &matched) > before,
            "the genuine match is still strengthened"
        );
    }

    fn stability_of(conn: &rusqlite::Connection, id: &str) -> f64 {
        conn.query_row(
            "SELECT stability FROM solutions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
    }

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

    fn hit(id: &str, vec: Option<f32>, bm25: Option<f32>) -> RawHit {
        RawHit {
            id: id.into(),
            vec_score: vec,
            bm25_score: bm25,
        }
    }

    #[test]
    fn rrf_rank1_both_lists_scores_two() {
        let fused = fuse_rrf(
            &[
                hit("a", Some(1.0), Some(10.0)),
                hit("b", Some(0.0), Some(0.0)),
            ],
            RRF_K,
        );
        assert_eq!(fused[0].0, "a");
        assert!((fused[0].1 - 2.0).abs() < 1e-5);
        let rank2 = 2.0 * (RRF_K + 1.0) / (RRF_K + 2.0);
        assert!((fused[1].1 - rank2).abs() < 1e-5);
    }

    #[test]
    fn rrf_prefers_consensus_over_single_list() {
        let fused = fuse_rrf(
            &[hit("a", Some(1.0), None), hit("b", Some(0.0), Some(5.0))],
            RRF_K,
        );
        let map: HashMap<_, _> = fused.into_iter().collect();
        // a: vec rank 1 only → 1.0
        // b: vec rank 2 + bm25 rank 1 → (k+1)/(k+2) + 1
        assert!((map["a"] - 1.0).abs() < 1e-5);
        assert!(map["b"] > map["a"]);
        assert!((map["b"] - (1.0 + (RRF_K + 1.0) / (RRF_K + 2.0))).abs() < 1e-5);
    }

    #[test]
    fn rrf_missing_path_is_one_list_only() {
        let fused = fuse_rrf(
            &[hit("a", Some(1.0), None), hit("b", None, Some(5.0))],
            RRF_K,
        );
        let map: HashMap<_, _> = fused.into_iter().collect();
        assert!((map["a"] - 1.0).abs() < 1e-5);
        assert!((map["b"] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn fuse_dispatches_rrf() {
        let hits = [hit("a", Some(1.0), Some(10.0)), hit("b", Some(0.0), Some(0.0))];
        let summed = fuse(&hits, Fusion::Sum);
        let rrfd = fuse(&hits, Fusion::Rrf);
        assert_eq!(summed[0].0, "a");
        assert_eq!(rrfd[0].0, "a");
        assert!((summed[0].1 - 2.0).abs() < 1e-5);
        assert!((rrfd[0].1 - 2.0).abs() < 1e-5);
    }
}
