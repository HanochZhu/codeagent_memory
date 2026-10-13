//! Revision-chain expansion: pull the newest revision of a matched memory in.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::fuse::sort_desc;

/// Fraction of the fused score a revision tail inherits from the seed that pulled
/// it in, so revision expansion alone does not look like a strong topical match.
const EXPAND_DECAY: f32 = 0.5;

/// Revision tails worth adding to the fused list, scored off the top `seeds`.
///
/// A revision is stored as a new row rather than an edit, so the newest node
/// on a chain often shares no wording with the query and neither the vector nor
/// the BM25 path can reach it. Pulling the tail in keeps the current answer
/// reachable even when the query only matches a revision several steps behind.
pub(super) fn revision_tails(
    conn: &Connection,
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
pub(super) struct RevisionIndex {
    tails: HashMap<String, String>,
}

impl RevisionIndex {
    pub(super) fn tail_of(&mut self, conn: &Connection, id: &str) -> Result<String> {
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
fn revision_chain(conn: &Connection, id: &str) -> Result<Vec<String>> {
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

#[cfg(test)]
mod tests {
    use super::super::testing::{add, revise};
    use super::*;
    use crate::project::Project;
    use tempfile::tempdir;

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
}
