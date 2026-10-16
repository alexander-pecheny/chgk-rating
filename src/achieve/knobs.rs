//! The rules of the aggregate, every one a knob: what a result is worth, how it
//! fades, how many count and who is listed.

/// The choices of the knobs that take a word.
const PERFS: [&str; 5] = ["strength_place", "score", "place", "mix", "beaten"];
const FIELD_MODES: [&str; 3] = ["sum", "world", "anchors"];
const FADES: [&str; 4] = ["none", "linear", "exp", "yearly"];
const OVERLAP_BY: [&str; 2] = ["strength", "count"];
/// The bounds `set` holds the counting knobs to.
const MAX_FIELD_K: usize = 50;
const MAX_TOP_X: usize = 100;

#[derive(Clone, Debug)]
pub struct Knobs {
    /// "strength_place": the score-based strength over place^place_pow;
    /// "score", "place" or "mix": `mix` of score and 1 - mix of place;
    /// "beaten": the mean lineup strength of the `beat_k` strongest teams it
    /// placed above, empty slots at `beat_fill`, plus `shift`, over place^place_pow.
    pub perf: &'static str,
    pub place_pow: f64,
    pub mix: f64,
    /// Added to the score-based number to put it on the lineup scale.
    pub shift: f64,
    pub w_sync: f64,
    pub w_strict: f64,
    pub w_async: f64,
    /// "world": the field's top `field_k` lineups over the season's top 20 base
    /// rosters that week, capped at 1; "anchors": the mean of the top
    /// `field_k` lineups placed between `field_zero` and `field_ref`.
    pub field_mode: &'static str,
    /// Field strength is read over the top `field_k` lineups, empty seats at 0.
    pub field_k: usize,
    pub field_zero: f64,
    pub field_ref: f64,
    pub field_gamma: f64,
    pub field_min: f64,
    /// "anchors": read a field of fewer than `field_k` lineups over the empty
    /// seats too, at 0, when set; over its own lineups when not, and then
    /// times (lineups / field_k)^size_pow for the field's size.
    pub field_pad: bool,
    pub size_pow: f64,
    /// "strength_place": a result with no per-question data, and so no
    /// score-based strength, falls back on its place-based strength.
    pub fallback: bool,
    /// The place-based strength put on the score scale: fb_a + fb_b p +
    /// fb_place ln place + fb_n ln teams. Fitted on the synchronous top 30 with
    /// both numbers; 0, 1, 0, 0 is the raw number, which sat 1200 points high
    /// mid-table and 1900 on a win.
    pub fb_a: f64,
    pub fb_b: f64,
    pub fb_place: f64,
    pub fb_n: f64,
    /// Fall back only on results before this year, when masks are lost for
    /// good; a later result without one waits for it. 0 falls back always.
    pub fb_until: i32,
    pub beat_k: usize,
    pub beat_fill: f64,
    /// Empty slots at the weakest lineup beaten, if below `beat_fill`.
    pub beat_worst: bool,
    /// Each beaten lineup times (1 + d)^margin_pow (1 + margin_lin d), d the
    /// questions it finished behind.
    pub margin_pow: f64,
    pub margin_lin: f64,
    /// Count d per 36 questions of the package, when set.
    pub margin_norm: bool,
    /// The margin worth 1: the factor is read over ((1 + d) / (1 + margin_ref)),
    /// so the ratings stay on the scale the fade's slow_ref assumes.
    pub margin_ref: f64,
    /// Read lineups under the forecast's seat weights, not chgk.gg's 6..1.
    pub q_fc: bool,
    /// "sum" mode, in person: (the sum of every lineup's strength / sum_ref)
    /// ^ sum_gamma, uncapped, so a field the size of the biggest festivals
    /// weighs about 1 and a bigger or stronger one more.
    pub sum_ref: f64,
    pub sum_gamma: f64,
    /// The published rating is the mean of the current values times this; 0
    /// picks it so the top 100 average what their counted results demonstrated.
    pub scale: f64,
    /// In person only: times (countries in the field)^country_pow and
    /// (towns in the field)^town_pow, a country or town counting once it
    /// sends `country_min` / `town_min` teams.
    pub country_pow: f64,
    pub country_min: usize,
    pub town_pow: f64,
    pub town_min: usize,
    /// The counts that weigh 1: the factor is (countries / country_ref)^pow
    /// times (towns / town_ref)^pow, capped at 1 when `geo_cap`, so a local
    /// field is discounted and a wide one never multiplied.
    pub country_ref: f64,
    pub town_ref: f64,
    pub geo_cap: bool,
    /// "linear" to zero at `zero_days`, "exp" halving every `half_life` days
    /// and dropped at `tail_days`, "yearly" losing a share a year, or "none":
    /// a result keeps its value forever.
    pub fade: &'static str,
    pub full_days: i32,
    pub zero_days: i32,
    /// The synchronous fade; `sync_zero` -1 is the in-person one.
    pub sync_full: i32,
    pub sync_zero: i32,
    pub half_life: f64,
    pub sync_half_life: f64,
    /// "yearly": the share of value lost per year of age, from the first day.
    pub loss_ip: f64,
    pub loss_sync: f64,
    pub tail_days: i32,
    /// A strong result fades slower: its age runs at (slow_ref / value)^slow_k.
    pub slow_ref: f64,
    pub slow_k: f64,
    pub top_x: usize,
    /// The divisor is the slots held (a faded result holds its faded share),
    /// never below this; 0 divides by `top_x` always.
    pub min_slots: usize,
    /// This many of the top_x slots go to in-person results only, and a team
    /// with fewer holds zero in the rest of them; 0 is off.
    pub min_ip: usize,
    /// The divisor is fixed on the day after the team's last result, and the
    /// same results keep fading under it, so an idle team can only fall.
    pub freeze: bool,
    /// Count a result only when the lineup is the team's own by the MAII rule.
    pub base_rule: bool,
    /// Days into a season the last season's roster still lists a team; -1 lists
    /// every team whatever its roster.
    pub grace: i32,
    /// A team is listed only with a counted result in the last `active_days`
    /// days; 0 lists it however long it has been idle.
    pub active_days: i32,
    /// Scale a result by the share of that season's base roster still on the
    /// team's current one, to this power; 0 is off.
    pub overlap_k: f64,
    /// "strength": the kept players' share of the roster's rating points;
    /// "count": their share of its seats.
    pub overlap_by: &'static str,
    /// Published points added to a sole first place before the weights.
    pub win_bonus: f64,
    /// A result's value times Q(L) of the season's base roster over Q(L) of the
    /// lineup that played, when the lineup was the stronger: legionnaires who
    /// lift a team above its base roster do not lift its rating as far.
    pub legion: bool,
    /// The published rating is strength_a·points^strength_p: the strength a
    /// team's counted results show, on the lineup scale. Fitted once on the top
    /// 100 of the releases of 2016-2024 and fixed, so a team's number never
    /// moves because others' did; strength_p = 0 publishes the points.
    pub strength_a: f64,
    pub strength_p: f64,
    /// The published rating is the sum of the counted results, points times
    /// top_x, and strength_a and strength_p are not read. With min_slots =
    /// top_x the divisor is always top_x, so this is the sum exactly.
    pub sum: bool,
}

/// The variant on the site: the user's constraints (seven results,
/// synchronous 0.5, strictly synchronous 2/3 as in the official rules, a divisor that never lets an
/// idle rating grow) and the rest from `achieve --search --fix`.
const DEFAULT: Knobs = Knobs {
    perf: "beaten",
    place_pow: 0.8,
    mix: 0.5,
    // Beaten opponents are on the lineup scale already; -800 measured within noise.
    shift: 0.0,
    w_sync: 0.5,
    w_strict: 2.0 / 3.0,
    w_async: 0.0,
    field_mode: "anchors",
    field_k: 20,
    field_zero: 5000.0,
    field_ref: 10000.0,
    field_gamma: 1.0,
    field_min: 0.0,
    field_pad: false,
    size_pow: 0.5,
    fallback: true,
    fb_a: -1561.9,
    fb_b: 1.0513,
    fb_place: 252.8,
    fb_n: -201.1,
    fb_until: 2020,
    beat_k: 10,
    beat_fill: 5000.0,
    // Both on since 1.5: r +0.0010 +/- 0.0005 on 2018-09-01..2024-12-31 and
    // +0.0014 +/- 0.0006 on 2025-01-01..2026-08-27, NDCG@3 -0.0027 +/- 0.0015
    // and +0.0005 +/- 0.0012.
    beat_worst: true,
    // Measured and off: (1 + d)^0.7 gains r 0.0097 +/- 0.0024 on tune only by
    // raising every value sevenfold, which slows the fade and shrinks the fill;
    // at margin_ref 15, on today's scale, test NDCG -0.011 +/- 0.005 and r
    // -0.014 +/- 0.003. margin_lin and margin_norm do no better.
    margin_pow: 0.0,
    margin_lin: 0.0,
    margin_norm: false,
    margin_ref: 0.0,
    q_fc: true,
    sum_ref: 1_000_000.0,
    sum_gamma: 0.5,
    scale: 1.0,
    country_pow: 0.0,
    country_min: 1,
    town_pow: 0.6,
    town_min: 1,
    country_ref: 1.0,
    town_ref: 24.0,
    geo_cap: true,
    fade: "yearly",
    full_days: 365,
    zero_days: 730,
    sync_full: 90,
    sync_zero: 365,
    half_life: 180.0,
    sync_half_life: 137.0,
    loss_ip: 0.1,
    loss_sync: 0.4,
    tail_days: 1826,
    slow_ref: 9000.0,
    slow_k: 0.6,
    top_x: 7,
    min_slots: 7,
    min_ip: 0,
    freeze: true,
    base_rule: false,
    grace: 60,
    active_days: 365,
    overlap_k: 0.6,
    overlap_by: "strength",
    win_bonus: 0.0,
    legion: true,
    strength_a: 4520.0,
    strength_p: 0.0956,
    sum: true,
};

impl Default for Knobs {
    fn default() -> Knobs {
        DEFAULT
    }
}

/// Every knob by key, in the order the site lists the rules.
pub const KNOBS: &[&str] = &[
    "top_x", "min_slots", "min_ip", "freeze", "perf", "place_pow", "mix", "shift", "w_sync", "w_strict", "w_async",
    "field_mode", "scale", "sum_ref", "sum_gamma", "field_k", "field_zero", "field_ref", "field_gamma", "field_min", "field_pad", "size_pow", "fallback", "fb_a", "fb_b", "fb_place", "fb_n", "fb_until", "beat_k", "beat_fill", "beat_worst", "margin_pow", "margin_lin", "margin_norm", "margin_ref", "q_fc",
    "country_pow", "country_min", "town_pow", "town_min", "country_ref", "town_ref", "geo_cap", "fade", "loss_ip",
    "loss_sync", "full_days", "zero_days", "half_life", "sync_full", "sync_zero", "sync_half_life", "tail_days",
    "slow_k", "slow_ref", "base_rule", "grace", "active_days", "overlap_k", "overlap_by", "win_bonus", "legion", "strength_a", "strength_p", "sum",
];

/// `val` as one of `opts`, or the error the lab shows.
fn choice(key: &str, opts: &[&'static str], val: &str) -> Result<&'static str, String> {
    opts.iter().find(|&&o| o == val).copied().ok_or(format!("{key}: нет такого значения: {val}"))
}

impl Knobs {
    pub fn get(&self, key: &str) -> String {
        match key {
            "perf" => self.perf.to_string(),
            "place_pow" => self.place_pow.to_string(),
            "field_mode" => self.field_mode.to_string(),
            "sum_ref" => self.sum_ref.to_string(),
            "scale" => self.scale.to_string(),
            "sum_gamma" => self.sum_gamma.to_string(),
            "mix" => self.mix.to_string(),
            "shift" => self.shift.to_string(),
            "w_sync" => self.w_sync.to_string(),
            "w_strict" => self.w_strict.to_string(),
            "w_async" => self.w_async.to_string(),
            "field_k" => self.field_k.to_string(),
            "field_zero" => self.field_zero.to_string(),
            "field_ref" => self.field_ref.to_string(),
            "field_gamma" => self.field_gamma.to_string(),
            "field_min" => self.field_min.to_string(),
            "field_pad" => (self.field_pad as u8).to_string(),
            "size_pow" => self.size_pow.to_string(),
            "fallback" => (self.fallback as u8).to_string(),
            "fb_a" => self.fb_a.to_string(),
            "fb_b" => self.fb_b.to_string(),
            "fb_place" => self.fb_place.to_string(),
            "fb_n" => self.fb_n.to_string(),
            "fb_until" => self.fb_until.to_string(),
            "beat_k" => self.beat_k.to_string(),
            "beat_fill" => self.beat_fill.to_string(),
            "beat_worst" => (self.beat_worst as u8).to_string(),
            "margin_pow" => self.margin_pow.to_string(),
            "margin_lin" => self.margin_lin.to_string(),
            "margin_norm" => (self.margin_norm as u8).to_string(),
            "margin_ref" => self.margin_ref.to_string(),
            "q_fc" => (self.q_fc as u8).to_string(),
            "country_pow" => self.country_pow.to_string(),
            "country_min" => self.country_min.to_string(),
            "town_pow" => self.town_pow.to_string(),
            "town_min" => self.town_min.to_string(),
            "country_ref" => self.country_ref.to_string(),
            "town_ref" => self.town_ref.to_string(),
            "geo_cap" => (self.geo_cap as u8).to_string(),
            "fade" => self.fade.to_string(),
            "full_days" => self.full_days.to_string(),
            "zero_days" => self.zero_days.to_string(),
            "sync_full" => self.sync_full.to_string(),
            "sync_zero" => self.sync_zero.to_string(),
            "half_life" => self.half_life.to_string(),
            "loss_ip" => self.loss_ip.to_string(),
            "loss_sync" => self.loss_sync.to_string(),
            "sync_half_life" => self.sync_half_life.to_string(),
            "tail_days" => self.tail_days.to_string(),
            "slow_ref" => self.slow_ref.to_string(),
            "slow_k" => self.slow_k.to_string(),
            "top_x" => self.top_x.to_string(),
            "min_slots" => self.min_slots.to_string(),
            "min_ip" => self.min_ip.to_string(),
            "freeze" => (self.freeze as u8).to_string(),
            "base_rule" => (self.base_rule as u8).to_string(),
            "grace" => self.grace.to_string(),
            "active_days" => self.active_days.to_string(),
            "overlap_k" => self.overlap_k.to_string(),
            "overlap_by" => self.overlap_by.to_string(),
            "win_bonus" => self.win_bonus.to_string(),
            "legion" => (self.legion as u8).to_string(),
            "strength_a" => self.strength_a.to_string(),
            "strength_p" => self.strength_p.to_string(),
            "sum" => (self.sum as u8).to_string(),
            _ => String::new(),
        }
    }

    pub fn set(&mut self, key: &str, val: &str) -> Result<(), String> {
        let val = val.trim();
        match key {
            "perf" => self.perf = choice(key, &PERFS, val)?,
            "field_mode" => self.field_mode = choice(key, &FIELD_MODES, val)?,
            "fade" => self.fade = choice(key, &FADES, val)?,
            "overlap_by" => self.overlap_by = choice(key, &OVERLAP_BY, val)?,
            _ if !KNOBS.contains(&key) => return Err(format!("нет такой ручки: {key}")),
            _ => {
                let x = val.parse::<f64>().map_err(|_| format!("{key}: не число: {val}"))?;
                self.set_number(key, x)?;
            }
        }
        Ok(())
    }

    /// The knobs that take a number, each held to its range.
    fn set_number(&mut self, key: &str, x: f64) -> Result<(), String> {
        match key {
            "place_pow" => self.place_pow = x,
            "sum_ref" => self.sum_ref = x.max(1.0),
            "scale" => self.scale = x.max(0.0),
            "sum_gamma" => self.sum_gamma = x,
            "mix" => self.mix = x.clamp(0.0, 1.0),
            "shift" => self.shift = x,
            "w_sync" => {
                self.w_sync = x;
                self.w_strict = x;
            }
            "w_strict" => self.w_strict = x,
            "w_async" => self.w_async = x,
            "field_k" => self.field_k = (x as usize).clamp(1, MAX_FIELD_K),
            "field_zero" => self.field_zero = x,
            "field_ref" => self.field_ref = x,
            "field_gamma" => self.field_gamma = x,
            "field_min" => self.field_min = x,
            "field_pad" => self.field_pad = x != 0.0,
            "size_pow" => self.size_pow = x,
            "fb_a" => self.fb_a = x,
            "fb_b" => self.fb_b = x,
            "fb_place" => self.fb_place = x,
            "fb_n" => self.fb_n = x,
            "fb_until" => self.fb_until = x as i32,
            "beat_k" => self.beat_k = (x as usize).max(1),
            "beat_fill" => self.beat_fill = x,
            "beat_worst" => self.beat_worst = x != 0.0,
            "margin_pow" => self.margin_pow = x,
            "margin_lin" => self.margin_lin = x,
            "margin_norm" => self.margin_norm = x != 0.0,
            "margin_ref" => self.margin_ref = x.max(0.0),
            "q_fc" => self.q_fc = x != 0.0,
            "fallback" => self.fallback = x != 0.0,
            "country_pow" => self.country_pow = x,
            "country_min" => self.country_min = (x as usize).max(1),
            "town_pow" => self.town_pow = x,
            "town_min" => self.town_min = (x as usize).max(1),
            "country_ref" => self.country_ref = x.max(1.0),
            "town_ref" => self.town_ref = x.max(1.0),
            "geo_cap" => self.geo_cap = x != 0.0,
            "full_days" => self.full_days = x as i32,
            "zero_days" => self.zero_days = x as i32,
            "sync_full" => self.sync_full = x as i32,
            "sync_zero" => self.sync_zero = x as i32,
            "half_life" => self.half_life = x,
            "loss_ip" => self.loss_ip = x.clamp(0.0, 1.0),
            "loss_sync" => self.loss_sync = x.clamp(0.0, 1.0),
            "sync_half_life" => self.sync_half_life = x,
            "tail_days" => self.tail_days = x as i32,
            "slow_ref" => self.slow_ref = x,
            "slow_k" => self.slow_k = x,
            "top_x" => self.top_x = (x as usize).clamp(1, MAX_TOP_X),
            "min_slots" => self.min_slots = (x as usize).min(MAX_TOP_X),
            "min_ip" => self.min_ip = (x as usize).min(MAX_TOP_X),
            "freeze" => self.freeze = x != 0.0,
            "base_rule" => self.base_rule = x != 0.0,
            "grace" => self.grace = x as i32,
            "active_days" => self.active_days = x as i32,
            "overlap_k" => self.overlap_k = x,
            "win_bonus" => self.win_bonus = x,
            "legion" => self.legion = x != 0.0,
            "strength_a" => self.strength_a = x.max(0.0),
            "strength_p" => self.strength_p = x.clamp(0.0, 1.0),
            "sum" => self.sum = x != 0.0,
            _ => return Err(format!("нет такой ручки: {key}")),
        }
        Ok(())
    }

    /// `"top_x=8,fade=none"` on top of `self`.
    pub fn apply(&self, spec: &str) -> Result<Knobs, String> {
        let mut k = self.clone();
        for kv in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (key, val) = kv.split_once('=').ok_or(format!("expected key=value: {kv}"))?;
            k.set(key, val)?;
        }
        Ok(k)
    }
}
