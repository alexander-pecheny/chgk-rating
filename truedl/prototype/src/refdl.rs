//! The published TrueDL, reimplemented from count_truedl_single_tourn.py.

use crate::util::round1;

const KEYS: [i64; 10] = [10, 25, 50, 100, 250, 500, 1000, 2000, 3000, 5000];
const VALS: [f64; 10] = [1.61, 1.52, 1.43, 1.32, 1.16, 1.0, 0.81, 0.6, 0.43, 0.31];

/// bisect_left over KEYS, then VALS; None at place >= 5000 (as in the reference code).
pub fn coeff(place: f64) -> Option<f64> {
    let p = place.round() as i64;
    if p >= 5000 || p <= 0 {
        return None;
    }
    let mut i = 0;
    while i < KEYS.len() && KEYS[i] < p {
        i += 1;
    }
    if i >= VALS.len() {
        None
    } else {
        Some(VALS[i])
    }
}

/// variant 0: as in count_truedl_single_tourn.py (per-team value rounded to 0.1)
/// variant 1: same without any rounding
/// variant 2: as in 06_calculate_truedl.py (int() truncation of the hypothetical result)
pub fn team_truedl(q: f64, n: f64, place: f64, variant: u8) -> Option<f64> {
    let c = coeff(place)?;
    let hyp = match variant {
        2 => (q / c).trunc().min(n),
        _ => (q / c).min(n),
    };
    let v = (1.0 - hyp / n) * 10.0;
    Some(if variant == 0 { round1(v) } else { v })
}
