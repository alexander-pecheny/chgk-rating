//! Noisy-OR question model of DESIGN.md section 3, with m fixed at 1 (config s_m = 0).

pub const B_BAR: f64 = 1.5;
pub const S_B: f64 = 2.0;
pub const S_A: f64 = 0.5;
pub const A_MIN: f64 = 0.3;
pub const A_MAX: f64 = 3.0;
pub const B_MIN: f64 = -10.0;
pub const B_MAX: f64 = 12.0;
pub const REF_SIZE: f64 = 6.0;

/// One team's answer to one question: index into the theta table plus the outcome.
pub struct Obs {
    pub team: u32,
    pub y: u8,
}

struct Acc {
    f: f64,
    gb: f64,
    gl: f64,
    hbb: f64,
    hbl: f64,
    hll: f64,
}

fn eval(obs: &[Obs], thetas: &[Vec<f64>], w: f64, b: f64, l: f64, want_h: bool) -> Acc {
    let a = l.exp();
    let mut acc = Acc { f: 0.0, gb: 0.0, gl: 0.0, hbb: 0.0, hbl: 0.0, hll: 0.0 };
    for o in obs {
        let th = &thetas[o.team as usize];
        let mut lam = 0.0;
        let mut s1 = 0.0;
        let mut s2 = 0.0;
        for &t in th {
            let d = t - b;
            let li = (a * d).exp();
            lam += li;
            if want_h || true {
                s1 += li * d;
                s2 += li * d * d;
            }
        }
        if lam < 1e-12 {
            lam = 1e-12;
        }
        let tt = (-lam).exp(); // 1 - P
        let p = 1.0 - tt;
        let y = o.y as f64;
        let pc = p.max(1e-12);
        acc.f += w * (y * pc.ln() + (1.0 - y) * (-lam));
        let dl = y * tt / pc - (1.0 - y);
        let ddl = -y * tt / (pc * pc);
        let u = -a * lam;
        let v = a * s1;
        acc.gb += w * dl * u;
        acc.gl += w * dl * v;
        if want_h {
            let lbb = a * a * lam;
            let lbl = -a * (lam + v);
            let lll = v + a * a * s2;
            acc.hbb += w * (ddl * u * u + dl * lbb);
            acc.hbl += w * (ddl * u * v + dl * lbl);
            acc.hll += w * (ddl * v * v + dl * lll);
        }
    }
    // priors
    acc.f += -(b - B_BAR) * (b - B_BAR) / (2.0 * S_B * S_B) - l * l / (2.0 * S_A * S_A);
    acc.gb += -(b - B_BAR) / (S_B * S_B);
    acc.gl += -l / (S_A * S_A);
    if want_h {
        acc.hbb += -1.0 / (S_B * S_B);
        acc.hll += -1.0 / (S_A * S_A);
    }
    acc
}

/// Newton on the penalised log-likelihood in (b, log a). Returns (b, a).
pub fn fit_question(obs: &[Obs], thetas: &[Vec<f64>], w: f64) -> (f64, f64) {
    if obs.is_empty() {
        return (B_BAR, 1.0);
    }
    let lmin = A_MIN.ln();
    let lmax = A_MAX.ln();
    let mut b = B_BAR;
    let mut l = 0.0f64;
    let mut cur = eval(obs, thetas, w, b, l, true);
    for _ in 0..15 {
        let det = cur.hbb * cur.hll - cur.hbl * cur.hbl;
        let (mut db, mut dl);
        if det > 1e-12 && cur.hbb < 0.0 {
            db = -(cur.hll * cur.gb - cur.hbl * cur.gl) / det;
            dl = -(-cur.hbl * cur.gb + cur.hbb * cur.gl) / det;
        } else {
            db = 0.1 * cur.gb;
            dl = 0.1 * cur.gl;
        }
        let n = (db * db + dl * dl).sqrt();
        if n > 2.0 {
            db *= 2.0 / n;
            dl *= 2.0 / n;
        }
        let mut step = 1.0;
        let mut moved = false;
        for _ in 0..6 {
            let nb = (b + step * db).clamp(B_MIN, B_MAX);
            let nl = (l + step * dl).clamp(lmin, lmax);
            let cand = eval(obs, thetas, w, nb, nl, true);
            if cand.f > cur.f {
                let d = ((nb - b).powi(2) + (nl - l).powi(2)).sqrt();
                b = nb;
                l = nl;
                cur = cand;
                moved = true;
                if d < 1e-4 {
                    return (b, l.exp());
                }
                break;
            }
            step *= 0.5;
        }
        if !moved {
            break;
        }
    }
    (b, l.exp())
}

/// Probability that a lineup of `REF_SIZE` identical players of strength theta solves q.
#[inline]
pub fn p_ref(theta: f64, b: f64, a: f64, m: f64) -> f64 {
    let lam = REF_SIZE * (a * (theta - b)).exp();
    let p = 1.0 - (-lam).exp();
    if m == 1.0 {
        p
    } else {
        p.powf(m)
    }
}

/// Expected share of the package solved by the reference lineup.
pub fn ref_rate(theta: f64, b: &[f64], a: &[f64], m: &[f64]) -> f64 {
    let mut s = 0.0;
    for i in 0..b.len() {
        s += p_ref(theta, b[i], a[i], m[i]);
    }
    s / b.len() as f64
}

/// Strength at which the reference lineup solves half the package.
pub fn theta_half(b: &[f64], a: &[f64], m: &[f64]) -> f64 {
    let (mut lo, mut hi) = (-8.0f64, 14.0f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if ref_rate(mid, b, a, m) < 0.5 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}
