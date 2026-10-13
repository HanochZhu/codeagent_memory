//! Reciprocal Rank Fusion of the vector and BM25 paths.

use std::collections::HashMap;

/// One scored memory from one recall path, before fusion.
#[derive(Debug, Clone)]
pub struct RawHit {
    pub id: String,
    pub vec_score: Option<f32>,
    pub bm25_score: Option<f32>,
}

/// Cormack et al. RRF constant. Raw RRF is scaled by `(k + 1)` so a rank-1
/// hit on one list scores 1.0 and a rank-1 hit on both lists scores 2.0
/// before Ebbinghaus retention scales it.
pub(super) const RRF_K: f32 = 60.0;

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

fn add_rrf_ranks(scores: &mut HashMap<String, f32>, ranked: &[(String, f32)], k: f32) {
    for (rank, (id, _)) in ranked.iter().enumerate() {
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + rank as f32 + 1.0);
    }
}

/// Sort `(id, score)` pairs by score, highest first.
pub(super) fn sort_desc(items: &mut [(String, f32)]) {
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
}

/// Drop entries below `ratio` of the best one. `items` must be sorted
/// descending. The best entry always stays, even when it is not positive.
pub(super) fn keep_near_top(items: &mut Vec<(String, f32)>, ratio: f32) {
    let Some(&(_, top)) = items.first() else {
        return;
    };
    if top <= 0.0 {
        items.truncate(1);
        return;
    }
    items.retain(|(_, s)| *s >= top * ratio);
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn keep_near_top_keeps_the_best_even_when_not_positive() {
        let mut items = vec![("a".to_string(), 2.0), ("b".into(), 0.7), ("c".into(), 0.5)];
        keep_near_top(&mut items, 0.3);
        assert_eq!(items.len(), 2);

        let mut flat = vec![("a".to_string(), 0.0), ("b".into(), 0.0)];
        keep_near_top(&mut flat, 0.3);
        assert_eq!(flat, vec![("a".to_string(), 0.0)]);
    }
}
