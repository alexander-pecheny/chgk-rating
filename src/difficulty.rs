//! A sample of the difficulties a field of a given strength usually gets.
//!
//! With `m ≠ 1` the ranking of a star-plus-passengers lineup against a balanced
//! one depends on how hard the package is, so `Q(L)` — one number per lineup,
//! with no package in it — stops being the right forecast. What replaces it is
//! the **expected number of answers** on a package of the difficulty this field
//! usually gets:
//!
//! ```text
//! forecast(L) = mean over the sample of b  of  (1 − exp(−Σ_{i∈L} e^{a(z_i − b)}))^m
//! ```
//!
//! The sample is drawn from the difficulties already fitted for in-person
//! tournaments of a similar field strength, and a tournament is added to it only
//! after it has been scored, so nothing a field carries enters its own forecast.

/// Cut points of the field-strength buckets, in display points: the mean lineup
/// strength of a field's best ten. The extremes are open.
const CUTS: [f64; 4] = [6400.0, 6900.0, 7400.0, 7900.0];
/// Difficulties kept per bucket; a new one displaces the oldest.
const CAP: usize = 4096;

pub struct Difficulties {
    buckets: Vec<Vec<f64>>,
    cursor: Vec<usize>,
    on: bool,
}

fn bucket_of(strength: f64) -> usize {
    CUTS.iter().filter(|&&c| strength >= c).count()
}

impl Difficulties {
    pub fn new(on: bool) -> Difficulties {
        Difficulties {
            buckets: vec![Vec::new(); CUTS.len() + 1],
            cursor: vec![0; CUTS.len() + 1],
            on,
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// The difficulties to score a field of this strength against: its own
    /// bucket when that has enough of them, otherwise everything collected so
    /// far. Empty until the first in-person tournament has been fitted.
    pub fn sample(&self, strength: f64) -> &[f64] {
        if !self.on {
            return &[];
        }
        let b = &self.buckets[bucket_of(strength)];
        if b.len() >= 256 {
            return b;
        }
        // Too thin to stand on its own: fall back to the fullest neighbour.
        self.buckets
            .iter()
            .max_by_key(|v| v.len())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Adds the difficulties of one scored in-person tournament.
    pub fn absorb(&mut self, strength: f64, bs: impl Iterator<Item = f64>) {
        if !self.on {
            return;
        }
        let k = bucket_of(strength);
        for b in bs.filter(|b| b.is_finite()) {
            if self.buckets[k].len() < CAP {
                self.buckets[k].push(b);
            } else {
                let c = self.cursor[k] % CAP;
                self.buckets[k][c] = b;
                self.cursor[k] = c + 1;
            }
        }
    }
}
