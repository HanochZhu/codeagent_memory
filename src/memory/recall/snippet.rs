//! Snippet clipping: long bodies below the top hit return a snippet.

use super::RecallHit;

/// Share of the top hit's relevance a hit needs to keep its full body. RRF is
/// flat (rank 2 on one list is 0.98 of rank 1), so only near-ties with the top
/// qualify; the rest return a snippet and are read in full with `mem show`.
const FULL_BODY_RATIO: f32 = 0.99;
const SNIPPET_CHARS: usize = 240;
/// Bodies up to this size stay whole: clipping them saves less than the
/// `mem show` round trip an agent pays when it needs the rest.
const CLIP_MIN_CHARS: usize = 4 * SNIPPET_CHARS;

/// `body` clipped to `max` chars with a trailing ellipsis, or `None` when it fits.
pub fn clip_body(body: &str, max: usize) -> Option<String> {
    if body.chars().count() <= max {
        return None;
    }
    let clipped: String = body.chars().take(max).collect();
    Some(format!("{}…", clipped.trim_end()))
}

/// Keep full bodies only for hits tied with the top one; clip the rest.
pub(super) fn clip_lower_bodies(hits: &mut [RecallHit]) {
    let Some(top) = hits.first().map(|h| h.relevance) else {
        return;
    };
    for hit in hits.iter_mut().skip(1) {
        if top > 0.0 && hit.relevance >= top * FULL_BODY_RATIO {
            continue;
        }
        if hit.body.chars().count() <= CLIP_MIN_CHARS {
            continue;
        }
        if let Some(snippet) = clip_body(&hit.body, SNIPPET_CHARS) {
            hit.body = snippet;
            hit.body_truncated = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::recall_hit;
    use super::*;

    #[test]
    fn only_hits_tied_with_the_top_keep_full_bodies() {
        let long = "word ".repeat(400);
        let medium = "word ".repeat(150);
        let mut hits = vec![
            recall_hit("top", 2.0, &long),
            recall_hit("tie", 1.984, &long),
            recall_hit("lower", 1.968, &long),
            recall_hit("medium", 1.0, &medium),
        ];
        clip_lower_bodies(&mut hits);

        assert!(!hits[0].body_truncated);
        assert_eq!(hits[0].body, long);
        assert!(!hits[1].body_truncated, "rank 1 + rank 2 is a near-tie");
        assert!(hits[2].body_truncated);
        assert!(hits[2].body.chars().count() <= SNIPPET_CHARS + 1);
        assert!(hits[2].body.ends_with('…'));
        assert!(
            !hits[3].body_truncated,
            "a body a mem show would cost more than it saves stays whole"
        );
    }
}
