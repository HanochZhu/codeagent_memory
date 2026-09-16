//! Ebbinghaus retention for memories.
//!
//! R = exp(-t / S) as used by MemoryBank (Zhong et al., 2023) and SuperMemo-style
//! stability. C0 is `recalled_at` (falls back to `created_at`).
//!
//! Thresholds (not FSRS's 0.9 review target — that is for *scheduling* reviews):
//! - initial S = 7 days
//! - on successful recall: refresh C0, S *= 1.7 (SM-2 default ease)
//! - needs_update when R < 0.3 (forgotten band in MemoryBank-style LLM memory)

pub const INITIAL_STABILITY_DAYS: f64 = 7.0;
pub const STABILITY_GROWTH: f64 = 1.7;
pub const FORGET_THRESHOLD: f64 = 0.3;
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Retrievability R in (0, 1].
pub fn retention(now: i64, c0: i64, stability_days: f64) -> f64 {
    let s = stability_days.max(0.1);
    let t_days = ((now - c0) as f64 / SECONDS_PER_DAY).max(0.0);
    (-t_days / s).exp()
}

pub fn c0(recalled_at: Option<i64>, created_at: i64) -> i64 {
    recalled_at.unwrap_or(created_at)
}

pub fn needs_update(r: f64) -> bool {
    r < FORGET_THRESHOLD
}

pub fn strengthen(stability_days: f64) -> f64 {
    (stability_days.max(INITIAL_STABILITY_DAYS)) * STABILITY_GROWTH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_memory_is_one() {
        assert!((retention(100, 100, 7.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn one_stability_period_is_e_inv() {
        let now = 0;
        let later = (INITIAL_STABILITY_DAYS * SECONDS_PER_DAY) as i64;
        let r = retention(later, now, INITIAL_STABILITY_DAYS);
        assert!((r - (-1.0f64).exp()).abs() < 1e-9);
    }

    #[test]
    fn forgotten_after_long_gap() {
        let now = 0;
        let later = (60.0 * SECONDS_PER_DAY) as i64;
        let r = retention(later, now, INITIAL_STABILITY_DAYS);
        assert!(needs_update(r));
    }

    #[test]
    fn strengthen_increases_stability() {
        assert!((strengthen(7.0) - 11.9).abs() < 1e-9);
    }
}
