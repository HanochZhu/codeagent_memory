use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde::Serialize;

use super::add::{escape_fts_query, require_english, tokenize_query};
use super::ebbinghaus::{c0, needs_update, retention, strengthen};
use super::embed::{cosine, decode_f32, Embedder};
use crate::config::Config;
use crate::project::Project;

const POOL: usize = 20;
/// Cormack et al. RRF constant. Raw RRF is scaled by `(k + 1)` so a rank-1
/// hit on one list scores 1.0 and a rank-1 hit on both lists scores 2.0
/// before Ebbinghaus retention scales it.
const RRF_K: f32 = 60.0;
/// Fraction of the fused score a revision tail inherits from the seed that pulled
/// it in, so revision expansion alone does not look like a strong topical match.
const EXPAND_DECAY: f32 = 0.5;
/// Share of a path's best score a memory needs to count as matched on that
/// path. RRF only sees ranks, so without a gate every memory the vector path
/// returns, and every document sharing one near-zero-IDF token such as `ts`,
/// scores almost like a real match.
const VEC_KEEP: f32 = 0.8;
const BM25_KEEP: f32 = 0.3;
/// Largest share of the final score Ebbinghaus retention can move. Retention
/// says how fresh a memory is, not whether it answers the query, so it must
/// not outweigh a difference in relevance.
const RETENTION_WEIGHT: f32 = 0.1;

#[derive(Debug, Clone)]
pub struct RawHit {
    pub id: String,
    pub vec_score: Option<f32>,
    pub bm25_score: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallHit {
    pub id: String,
    pub supersedes_id: Option<String>,
    pub summary: String,
    pub body: String,
    /// `relevance` scaled by retention; what results are sorted by.
    pub score: f32,
    /// Fused vector + BM25 score before retention.
    pub relevance: f32,
    pub vec_score: Option<f32>,
    pub bm25_score: Option<f32>,
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
    let mut fused: Vec<(String, f32)> = scores.into_iter().map(|(id, s)| (id, s * scale)).collect();
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

/// Drop entries below `ratio` of the best one. `items` must be sorted
/// descending. The best entry always stays, even when it is not positive.
fn keep_near_top(items: &mut Vec<(String, f32)>, ratio: f32) {
    let Some(&(_, top)) = items.first() else {
        return;
    };
    if top <= 0.0 {
        items.truncate(1);
        return;
    }
    items.retain(|(_, s)| *s >= top * ratio);
}

/// Knobs for one `recall` call.
#[derive(Debug, Clone, Copy)]
pub struct RecallOptions {
    pub limit: usize,
    /// Also pull in the newest revision of whatever the query matched, even
    /// when that revision matches neither the vector nor the BM25 path.
    pub expand: bool,
    /// Keep older revisions for history and comparison queries.
    pub include_superseded: bool,
}

impl Default for RecallOptions {
    fn default() -> Self {
        Self {
            limit: 3,
            expand: true,
            include_superseded: false,
        }
    }
}

pub fn recall(
    project: &Project,
    embedder: &dyn Embedder,
    query: &str,
    opts: RecallOptions,
) -> Result<Vec<RecallHit>> {
    if query.trim().is_empty() {
        anyhow::bail!("query is required");
    }
    require_english("query", query)?;
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
    sort_desc(&mut vec_hits);
    vec_hits.truncate(POOL);
    keep_near_top(&mut vec_hits, VEC_KEEP);

    let mut bm25_hits = Vec::new();
    let match_q = escape_fts_query(&tokenize_query(query));
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
                bm25_hits.push((id, -bm25 as f32));
            }
        }
    }
    keep_near_top(&mut bm25_hits, BM25_KEEP);

    let vec_of: HashMap<String, f32> = vec_hits.iter().cloned().collect();
    let bm25_of: HashMap<String, f32> = bm25_hits.iter().cloned().collect();
    let raw: Vec<RawHit> = vec_hits
        .into_iter()
        .map(|(id, s)| RawHit {
            id,
            vec_score: Some(s),
            bm25_score: None,
        })
        .chain(bm25_hits.into_iter().map(|(id, s)| RawHit {
            id,
            vec_score: None,
            bm25_score: Some(s),
        }))
        .collect();

    let fused = fuse_rrf(&raw, RRF_K);
    let direct_matches: HashSet<String> = fused.iter().map(|(id, _)| id.clone()).collect();
    let seeds = if opts.expand { opts.limit.max(1) } else { 0 };
    let mut revisions = RevisionIndex::default();
    let expanded = revision_tails(&conn, &mut revisions, &fused, seeds)?;
    let candidates: Vec<(String, f32)> = fused.into_iter().chain(expanded).collect();
    let candidate_ids: HashSet<String> = candidates.iter().map(|(id, _)| id.clone()).collect();
    let now = chrono::Utc::now().timestamp();
    let mut hits = Vec::new();
    for (id, relevance) in candidates {
        let (supersedes_id, summary, body, created_at, updated_at, recalled_at, stability): (
            Option<String>,
            String,
            String,
            i64,
            i64,
            Option<i64>,
            f64,
        ) = conn.query_row(
            "SELECT supersedes_id, summary, body, created_at, updated_at, recalled_at, stability
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
                    row.get(6)?,
                ))
            },
        )?;
        let r = retention(now, c0(recalled_at, created_at), stability);
        let tail = revisions.tail_of(&conn, &id)?;
        let latest = tail == id;
        if !opts.include_superseded && !latest && candidate_ids.contains(&tail) {
            continue;
        }
        hits.push(RecallHit {
            score: relevance * (1.0 - RETENTION_WEIGHT + RETENTION_WEIGHT * r as f32),
            relevance,
            vec_score: vec_of.get(&id).copied(),
            bm25_score: bm25_of.get(&id).copied(),
            id,
            supersedes_id,
            summary,
            body,
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
            .then_with(|| a.id.cmp(&b.id))
    });
    hits.truncate(opts.limit.max(1));

    for hit in &mut hits {
        hit.path = breadcrumb(&conn, &hit.id)?;
    }
    for hit in &hits {
        if !hit.needs_update && direct_matches.contains(&hit.id) {
            refresh_c0(&conn, &hit.id, now)?;
        }
    }
    Ok(hits)
}

/// Revision tails worth adding to the fused list, scored off the top `seeds`.
///
/// A revision is stored as a new row rather than an edit, so the newest node
/// on a chain often shares no wording with the query and neither the vector nor
/// the BM25 path can reach it. Pulling the tail in keeps the current answer
/// reachable even when the query only matches a revision several steps behind.
fn revision_tails(
    conn: &rusqlite::Connection,
    revisions: &mut RevisionIndex,
    fused: &[(String, f32)],
    seeds: usize,
) -> Result<Vec<(String, f32)>> {
    let matched: HashSet<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
    let mut added: HashMap<String, f32> = HashMap::new();

    for (id, score) in fused.iter().take(seeds) {
        let tail = revisions.tail_of(conn, id)?;
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

/// Memoised revision lookups. Structural parent/child relationships are not
/// consulted here and may branch freely.
#[derive(Default)]
struct RevisionIndex {
    tails: HashMap<String, String>,
}

impl RevisionIndex {
    fn tail_of(&mut self, conn: &rusqlite::Connection, id: &str) -> Result<String> {
        if let Some(tail) = self.tails.get(id) {
            return Ok(tail.clone());
        }
        let chain = revision_chain(conn, id)?;
        let tail = chain.last().cloned().unwrap_or_else(|| id.to_string());
        for member in chain {
            self.tails.insert(member, tail.clone());
        }
        Ok(tail)
    }
}

/// The linear revision suffix starting at `id`, in chronological order. A
/// unique index on `supersedes_id` guarantees at most one successor. The
/// visited set also protects recall if a database was modified outside the
/// append-only API.
fn revision_chain(conn: &rusqlite::Connection, id: &str) -> Result<Vec<String>> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = id.to_string();
    while seen.insert(current.clone()) {
        chain.push(current.clone());
        let next = conn
            .query_row(
                "SELECT id FROM solutions WHERE supersedes_id = ?1",
                [&current],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(next) = next else {
            break;
        };
        current = next;
    }
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
        add_solution(
            project,
            &HashEmbedder::default(),
            summary,
            body,
            parent,
            None,
        )
        .unwrap()
        .id
    }

    fn revise(project: &Project, summary: &str, body: &str, supersedes: &str) -> String {
        add_solution(
            project,
            &HashEmbedder::default(),
            summary,
            body,
            None,
            Some(supersedes),
        )
        .unwrap()
        .id
    }

    #[test]
    fn revision_tails_reaches_a_tail_two_hops_away() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let root = add(&project, "root", "root body", None);
        let seed = revise(&project, "seed", "seed body", &root);
        let tail = revise(&project, "tail", "tail body", &seed);

        let conn = project.connect().unwrap();
        let mut revisions = RevisionIndex::default();
        let tails = revision_tails(&conn, &mut revisions, &[(seed, 2.0)], 1).unwrap();

        assert_eq!(tails.len(), 1, "a superseded ancestor stays out: {tails:?}");
        assert_eq!(tails[0].0, tail);
        assert!((tails[0].1 - 1.0).abs() < 1e-5);
        assert_ne!(tails[0].0, root);
    }

    #[test]
    fn revision_tails_skips_a_tail_the_query_already_matched() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let root = add(&project, "root", "root body", None);
        let tail = revise(&project, "tail", "tail body", &root);

        let conn = project.connect().unwrap();
        let mut revisions = RevisionIndex::default();
        let tails = revision_tails(&conn, &mut revisions, &[(root, 2.0), (tail, 0.2)], 2).unwrap();

        assert!(tails.is_empty(), "{tails:?}");
    }

    #[test]
    fn zero_seeds_skips_expansion() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let root = add(&project, "root", "root body", None);
        revise(&project, "tail", "tail body", &root);

        let conn = project.connect().unwrap();
        let mut revisions = RevisionIndex::default();
        let tails = revision_tails(&conn, &mut revisions, &[(root, 2.0)], 0).unwrap();

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
        let mid = revise(
            &project,
            "retry policy rev1 exponential backoff",
            "retry policy rev1 exponential backoff",
            &root,
        );
        let tail = revise(
            &project,
            "retry policy rev2 jittered backoff",
            "retry policy rev2 jittered backoff",
            &mid,
        );

        let hits = recall(
            &project,
            &embedder,
            "retry policy backoff",
            RecallOptions {
                limit: 5,
                include_superseded: true,
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
    fn structural_children_are_independent_latest_memories() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let root = add(
            &project,
            "retry policy overview",
            "retry policy overview",
            None,
        );
        let network = add(
            &project,
            "network retry policy",
            "network retry policy",
            Some(&root),
        );
        let database = add(
            &project,
            "database retry policy",
            "database retry policy",
            Some(&root),
        );

        let hits = recall(
            &project,
            &embedder,
            "retry policy",
            RecallOptions {
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();

        for id in [&root, &network, &database] {
            let hit = hits.iter().find(|hit| hit.id == *id).unwrap();
            assert!(hit.latest, "structural siblings do not supersede: {hits:?}");
        }
    }

    #[test]
    fn history_mode_keeps_superseded_revisions() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let old = add(
            &project,
            "retry policy fixed backoff",
            "retry policy fixed backoff",
            None,
        );
        let latest = revise(
            &project,
            "retry policy jittered backoff",
            "retry policy jittered backoff",
            &old,
        );

        let hits = recall(
            &project,
            &embedder,
            "retry policy backoff",
            RecallOptions {
                limit: 5,
                include_superseded: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(hits.iter().any(|hit| hit.id == old && !hit.latest));
        assert!(hits.iter().any(|hit| hit.id == latest && hit.latest));
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
            "rank each path then apply reciprocal rank fusion",
            None,
        );
        let revision = revise(&project, "zzz", "zzz", &old);

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
            RecallOptions {
                limit: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        let pulled_in = hits
            .iter()
            .find(|h| h.id == revision)
            .expect("revision reached only through supersedes");
        assert!(pulled_in.latest);
        assert!(
            !hits.iter().any(|h| h.id == old),
            "superseded result should be folded before top-k: {hits:?}"
        );
    }

    #[test]
    fn an_expanded_revision_does_not_refresh_retention() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let matched = add(
            &project,
            "multi path recall fuses bm25 and vectors",
            "rank each path then apply reciprocal rank fusion",
            None,
        );
        let revision = revise(&project, "zzz", "zzz", &matched);

        let conn = project.connect().unwrap();
        conn.execute(
            "UPDATE solutions SET embedding = NULL, fts_text = '' WHERE id = ?1",
            [&revision],
        )
        .unwrap();
        let revision_before = stability_of(&conn, &revision);
        let matched_before = stability_of(&conn, &matched);
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
            (stability_of(&conn, &revision) - revision_before).abs() < 1e-9,
            "revision expansion alone must not strengthen a memory"
        );
        assert!(
            (stability_of(&conn, &matched) - matched_before).abs() < 1e-9,
            "a matched but hidden older revision must not be strengthened"
        );
    }

    #[test]
    fn retention_does_not_outrank_a_better_match() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let exact = add(
            &project,
            "bundledToolSchemas holds the five vfs tool schemas",
            "bundledToolSchemas.ts is kept in sync with toolMetadata.ts",
            None,
        );
        let hot = add(
            &project,
            "vfs_replace zeroes m_Script",
            "status missing maps to fileID 0 in yamlCompactInvert.ts",
            None,
        );
        // FTS5 clamps IDF to ~0 in a two-document store, which would make every
        // BM25 score equal; unrelated memories give the terms real weight.
        for topic in [
            "serve lock",
            "ndjson line limit",
            "query worker",
            "index build",
        ] {
            add(
                &project,
                topic,
                &format!("{topic} notes for the daemon"),
                None,
            );
        }
        let conn = project.connect().unwrap();
        conn.execute(
            "UPDATE solutions SET recalled_at = recalled_at - 20 * 86400 WHERE id = ?1",
            [&exact],
        )
        .unwrap();
        conn.execute(
            "UPDATE solutions SET stability = 7000 WHERE id = ?1",
            [&hot],
        )
        .unwrap();
        drop(conn);

        let hits = recall(
            &project,
            &embedder,
            "Is bundledToolSchemas.ts generated or updated by a script?",
            RecallOptions {
                limit: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(hits[0].id, exact, "{hits:?}");
        assert!(hits[0].retention < 0.1, "{hits:?}");
        if let Some(h) = hits.iter().find(|h| h.id == hot) {
            assert!(
                h.bm25_score.is_none(),
                "a shared `ts` is not a lexical match: {h:?}"
            );
        }
    }

    #[test]
    fn keep_near_top_keeps_the_best_even_when_not_positive() {
        let mut items = vec![("a".to_string(), 2.0), ("b".into(), 0.7), ("c".into(), 0.5)];
        keep_near_top(&mut items, 0.3);
        assert_eq!(items.len(), 2);

        let mut flat = vec![("a".to_string(), 0.0), ("b".into(), 0.0)];
        keep_near_top(&mut flat, 0.3);
        assert_eq!(flat, vec![("a".to_string(), 0.0)]);
    }

    fn stability_of(conn: &rusqlite::Connection, id: &str) -> f64 {
        conn.query_row(
            "SELECT stability FROM solutions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
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
}
