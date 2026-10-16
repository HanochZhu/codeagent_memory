//! Multi-path recall: run each path, fuse, expand, assemble.

mod bm25;
mod fuse;
mod hydrate;
mod reembed;
mod revision;
mod snippet;
mod vector;

pub use self::fuse::{fuse_rrf, RawHit};
pub use self::snippet::clip_body;

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde::Serialize;

use super::add::require_english;
use super::ebbinghaus::{c0, needs_update, retention};
use super::embed::Embedder;
use crate::config::Config;
use crate::project::Project;

use self::fuse::RRF_K;
use self::revision::RevisionIndex;

/// Candidates each recall path returns before fusion.
const POOL: usize = 20;
/// Largest share of the final score Ebbinghaus retention can move. Retention
/// says how fresh a memory is, not whether it answers the query, so it must
/// not outweigh a difference in relevance.
const RETENTION_WEIGHT: f32 = 0.1;

#[derive(Debug, Clone, Serialize)]
pub struct RecallHit {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes_id: Option<String>,
    pub summary: String,
    pub body: String,
    /// `body` is a snippet; `mem show <id>` returns the full text.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub body_truncated: bool,
    /// `relevance` scaled by retention; what results are sorted by.
    #[serde(serialize_with = "round2")]
    pub score: f32,
    /// Fused vector + BM25 score before retention.
    #[serde(skip)]
    pub relevance: f32,
    #[serde(skip)]
    pub vec_score: Option<f32>,
    #[serde(skip)]
    pub bm25_score: Option<f32>,
    pub path: Vec<String>,
    pub stale: bool,
    pub needs_update: bool,
    #[serde(serialize_with = "round2")]
    pub retention: f64,
    pub latest: bool,
    pub age_days: i64,
    #[serde(skip)]
    pub updated_at: i64,
}

fn round2<S: serde::Serializer, T: Copy + Into<f64>>(value: &T, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(((*value).into() * 100.0).round() / 100.0)
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
    /// Clip long bodies below the top hit to snippets. Off by default: agents
    /// rarely open a snippet, so clipping trades accuracy for tokens.
    pub snippets: bool,
}

impl Default for RecallOptions {
    fn default() -> Self {
        Self {
            limit: 3,
            expand: true,
            include_superseded: false,
            snippets: false,
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
    reembed::reembed_mismatched(&conn, embedder, &q_vec)?;

    let vec_hits = vector::vec_hits(&conn, embedder, &q_vec, POOL)?;
    let bm25_hits = bm25::bm25_hits(&conn, query, POOL)?;

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
    let expanded = revision::revision_tails(&conn, &mut revisions, &fused, seeds)?;
    let candidates: Vec<(String, f32)> = fused.into_iter().chain(expanded).collect();
    let candidate_ids: HashSet<String> = candidates.iter().map(|(id, _)| id.clone()).collect();
    let now = chrono::Utc::now().timestamp();

    let mut hits = Vec::new();
    for (id, relevance) in candidates {
        let row = hydrate::load_solution(&conn, &id)?;
        let r = retention(now, c0(row.recalled_at, row.created_at), row.stability);
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
            supersedes_id: row.supersedes_id,
            summary: row.summary,
            body: row.body,
            body_truncated: false,
            path: Vec::new(),
            stale: cfg.is_stale(row.updated_at),
            needs_update: needs_update(r),
            retention: r,
            latest,
            age_days: Config::age_days(row.updated_at),
            updated_at: row.updated_at,
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    hits.truncate(opts.limit.max(1));
    if opts.snippets {
        snippet::clip_lower_bodies(&mut hits);
    }

    for hit in &mut hits {
        hit.path = hydrate::breadcrumb(&conn, &hit.id)?;
    }
    for hit in &hits {
        if !hit.needs_update && direct_matches.contains(&hit.id) {
            hydrate::refresh_c0(&conn, &hit.id, now)?;
        }
    }
    Ok(hits)
}

/// Shared fixtures for the recall module's tests.
#[cfg(test)]
mod testing {
    use rusqlite::Connection;

    use super::RecallHit;
    use crate::memory::{add_solution, HashEmbedder};
    use crate::project::Project;

    pub fn add(project: &Project, summary: &str, body: &str, parent: Option<&str>) -> String {
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

    pub fn revise(project: &Project, summary: &str, body: &str, supersedes: &str) -> String {
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

    pub fn recall_hit(id: &str, relevance: f32, body: &str) -> RecallHit {
        RecallHit {
            id: id.into(),
            supersedes_id: None,
            summary: format!("{id} summary"),
            body: body.into(),
            body_truncated: false,
            score: relevance,
            relevance,
            vec_score: Some(0.5),
            bm25_score: None,
            path: Vec::new(),
            stale: false,
            needs_update: false,
            retention: 0.987654,
            latest: true,
            age_days: 0,
            updated_at: 0,
        }
    }

    pub fn stability_of(conn: &Connection, id: &str) -> f64 {
        conn.query_row(
            "SELECT stability FROM solutions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{add, recall_hit, revise, stability_of};
    use super::*;
    use crate::memory::HashEmbedder;
    use tempfile::tempdir;

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
    fn recall_json_drops_debug_scores_and_rounds() {
        let value = serde_json::to_value(recall_hit("a", 1.9672131, "body")).unwrap();
        let obj = value.as_object().unwrap();
        for gone in [
            "relevance",
            "vec_score",
            "bm25_score",
            "updated_at",
            "supersedes_id",
            "body_truncated",
        ] {
            assert!(!obj.contains_key(gone), "{gone} in {value}");
        }
        assert_eq!(value["score"], 1.97);
        assert_eq!(value["retention"], 0.99);
    }

    #[test]
    fn recall_returns_full_bodies_unless_snippets() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let filler = "unrelated filler text about other topics. ".repeat(40);
        add(
            &project,
            "retry policy backoff",
            &format!("retry policy backoff jitter {filler}"),
            None,
        );
        add(
            &project,
            "retry policy",
            &format!("retry policy {filler}"),
            None,
        );
        let embedder = HashEmbedder::default();
        let run = |snippets| {
            recall(
                &project,
                &embedder,
                "retry policy backoff jitter",
                RecallOptions {
                    limit: 5,
                    snippets,
                    ..Default::default()
                },
            )
            .unwrap()
        };

        let full = run(false);
        assert_eq!(full.len(), 2, "{full:?}");
        assert!(full.iter().all(|h| !h.body_truncated), "{full:?}");

        let clipped = run(true);
        assert!(!clipped[0].body_truncated);
        assert!(clipped[1].body_truncated, "{clipped:?}");
    }

    #[test]
    fn recall_reembeds_memories_from_another_model() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let embedder = HashEmbedder::default();
        let id = add(
            &project,
            "vector model identity",
            "stored embeddings must match the active embedding model",
            None,
        );
        let conn = project.connect().unwrap();
        let updated_at: i64 = conn
            .query_row(
                "SELECT updated_at FROM solutions WHERE id = ?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE solutions SET embedding_model = 'other-model' WHERE id = ?1",
            [&id],
        )
        .unwrap();
        drop(conn);

        let hits = recall(
            &project,
            &embedder,
            "vector model identity",
            RecallOptions {
                limit: 3,
                ..Default::default()
            },
        )
        .unwrap();
        let hit = hits.iter().find(|h| h.id == id).expect("memory still hits");
        assert!(hit.vec_score.is_some(), "{hit:?}");

        let conn = project.connect().unwrap();
        let (model, kept_updated_at): (String, i64) = conn
            .query_row(
                "SELECT embedding_model, updated_at FROM solutions WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(model, "hash");
        assert_eq!(kept_updated_at, updated_at);
    }

    #[test]
    fn recall_does_not_traverse_fact_links() {
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
        let linked = crate::memory::add_solution_with_links(
            &project,
            &embedder,
            "zzz",
            "zzz",
            None,
            None,
            std::slice::from_ref(&matched),
        )
        .unwrap()
        .id;
        let conn = project.connect().unwrap();
        conn.execute(
            "UPDATE solutions SET embedding = NULL, fts_text = '' WHERE id = ?1",
            [&linked],
        )
        .unwrap();
        drop(conn);

        let hits = recall(
            &project,
            &embedder,
            "bm25 and vectors",
            RecallOptions {
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(hits.iter().any(|h| h.id == matched), "{hits:?}");
        assert!(
            !hits.iter().any(|h| h.id == linked),
            "fact links must not trigger graph recall: {hits:?}"
        );
    }
}
