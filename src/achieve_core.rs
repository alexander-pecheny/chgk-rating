//! The achievement rating's aggregate: from each result's frozen values to a
//! team table, under a set of knobs. `src/bin/achieve.rs` writes the frozen
//! values; this module reads them, and so does the site's `/achieve/` lab
//! (`web/src/main.rs` includes this file by path), which is why it uses nothing
//! outside the standard library.
//!
//! ```text
//! perf    = how the team played: from the questions taken on the package
//!           (score), from the place against the field's lineups (place), or a
//!           blend of the two
//! value   = w_format · w_field · max(0, perf)
//! w_field = clamp(((top-k lineup mean − field_zero) / (field_ref − field_zero))^gamma, field_min, 1)
//! rating  = Σ best top_x of value · fade(age) / slots
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

// --------------------------------------------------------------------- dates

fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i32 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn parse_day(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i32> {
        let mut v = 0i32;
        for &c in &b[a..z] {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as i32;
        }
        Some(v)
    };
    Some(days_from_civil(num(0, 4)?, num(5, 7)? as u32, num(8, 10)? as u32))
}

pub fn show_day(z: i32) -> String {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    format!("{:04}-{m:02}-{d:02}", if m <= 2 { y + 1 } else { y })
}

// ------------------------------------------------------------ frozen values

/// One result, every strength on the published team scale. Nothing here
/// changes after the day it was played.
#[derive(Clone)]
pub struct Res {
    pub tid: i64,
    pub day: i32,
    /// 'i' in person, 's' synchronous, 'S' strictly synchronous, 'a' asynchronous.
    pub format: char,
    pub team: i64,
    pub place: f64,
    pub rated: bool,
    pub n: u32,
    pub n_base: u32,
    /// The lineup's strength as the forecast reads it; NaN with no lineup.
    pub q: f64,
    /// Performance from the questions taken, before `shift`.
    pub perf_s: f64,
    /// Performance from the place against the field's lineups.
    pub perf_p: f64,
    pub taken: f64,
    pub n_q: f64,
    /// The shipped fair rule's rating that morning, for the comparison.
    pub fair_mu: f64,
    /// Where it was played and the place there, when `tid` is a merged field.
    pub venue: i64,
    pub vplace: f64,
    /// In the team metric's qualifying set, and the tournament's metric weight.
    pub qual: bool,
    pub tw: f64,
    /// The combined strength of the season's 20 strongest base rosters that
    /// week, read the way this tournament's lineups are.
    pub ref20: f64,
    /// The team's town and country on the day, -1 where unknown.
    pub town: i64,
    pub country: i64,
}

const HEADER: &str = "tid\tday\tformat\tteam\tplace\trated\tn\tn_base\tq\tperf_s\tperf_p\ttaken\tn_q\tfair_mu\tvenue\tvplace\tqual\ttw\tref20\ttown\tcountry";

pub fn save_results(rs: &[Res], path: &str) -> std::io::Result<()> {
    let mut s = String::from(HEADER);
    s.push('\n');
    for r in rs {
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{}\t{}\t{:.1}\t{}\t{}\t{}\t{}\t{:.0}\t{}\t{}",
            r.tid,
            show_day(r.day),
            r.format,
            r.team,
            r.place,
            r.rated as u8,
            r.n,
            r.n_base,
            r.q,
            r.perf_s,
            r.perf_p,
            r.taken,
            r.n_q,
            r.fair_mu,
            r.venue,
            r.vplace,
            r.qual as u8,
            r.tw,
            r.ref20,
            r.town,
            r.country
        );
    }
    std::fs::write(path, s)
}

pub fn load_results(path: &str) -> std::io::Result<Vec<Res>> {
    let s = std::fs::read_to_string(path)?;
    let f = |x: &str| x.parse::<f64>().unwrap_or(f64::NAN);
    Ok(s.lines()
        .skip(1)
        .filter_map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            let tid: i64 = v[0].parse().ok()?;
            Some(Res {
                tid,
                day: parse_day(v[1])?,
                format: v[2].chars().next()?,
                team: v[3].parse().ok()?,
                place: f(v[4]),
                rated: v[5] == "1",
                n: v[6].parse().ok()?,
                n_base: v[7].parse().ok()?,
                q: f(v[8]),
                perf_s: f(v[9]),
                perf_p: f(v[10]),
                taken: f(v[11]),
                n_q: f(v[12]),
                fair_mu: f(v[13]),
                venue: v.get(14).and_then(|x| x.parse().ok()).unwrap_or(tid),
                vplace: v.get(15).map_or(f(v[4]), |x| f(x)),
                qual: v.get(16).is_some_and(|x| *x == "1"),
                tw: v.get(17).map_or(0.0, |x| f(x)),
                ref20: v.get(18).map_or(f64::NAN, |x| f(x)),
                town: v.get(19).and_then(|x| x.parse().ok()).unwrap_or(-1),
                country: v.get(20).and_then(|x| x.parse().ok()).unwrap_or(-1),
            })
        })
        .collect())
}

// ------------------------------------------------------------ names, rosters

/// Team and tournament names, and which teams had a base roster in which
/// season: `names.tsv` holds `T id name` and `U id name` lines, `rosters.tsv`
/// `S id first last` and `R team season` lines.
pub struct Names {
    pub teams: HashMap<i64, String>,
    pub tours: HashMap<i64, String>,
}

pub struct Seasons {
    pub list: Vec<(i64, i32, i32)>,
    pub has: HashSet<(i64, i64)>,
    /// The base roster of each (team, season), player ids sorted.
    pub players: HashMap<(i64, i64), Vec<i64>>,
    /// Each player's last published rating, clipped at 0.
    pub strength: HashMap<i64, f64>,
}

impl Names {
    pub fn load(path: &str) -> Names {
        let mut n = Names { teams: HashMap::new(), tours: HashMap::new() };
        for l in std::fs::read_to_string(path).unwrap_or_default().lines() {
            let mut it = l.splitn(3, '\t');
            let (Some(k), Some(id), Some(name)) = (it.next(), it.next(), it.next()) else { continue };
            let Ok(id) = id.parse() else { continue };
            match k {
                "T" => n.teams.insert(id, name.to_string()),
                "U" => n.tours.insert(id, name.to_string()),
                _ => None,
            };
        }
        n
    }
    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let mut s = String::new();
        for (id, name) in &self.teams {
            let _ = writeln!(s, "T\t{id}\t{}", name.replace(['\t', '\n'], " "));
        }
        for (id, name) in &self.tours {
            let _ = writeln!(s, "U\t{id}\t{}", name.replace(['\t', '\n'], " "));
        }
        std::fs::write(path, s)
    }
}

impl Seasons {
    pub fn load(path: &str) -> Seasons {
        let mut s = Seasons { list: Vec::new(), has: HashSet::new(), players: HashMap::new(), strength: HashMap::new() };
        for l in std::fs::read_to_string(path).unwrap_or_default().lines() {
            let v: Vec<&str> = l.split('\t').collect();
            match v.as_slice() {
                ["S", id, a, b] => {
                    if let (Ok(id), Some(a), Some(b)) = (id.parse(), parse_day(a), parse_day(b)) {
                        s.list.push((id, a, b));
                    }
                }
                ["R", t, x] => {
                    if let (Ok(t), Ok(x)) = (t.parse(), x.parse()) {
                        s.has.insert((t, x));
                    }
                }
                ["Q", pl, r] => {
                    if let (Ok(pl), Ok(r)) = (pl.parse::<i64>(), r.parse::<f64>()) {
                        s.strength.insert(pl, r.max(0.0));
                    }
                }
                ["P", t, x, pl] => {
                    if let (Ok(t), Ok(x), Ok(pl)) = (t.parse(), x.parse(), pl.parse()) {
                        s.players.entry((t, x)).or_default().push(pl);
                    }
                }
                _ => {}
            }
        }
        s.list.sort();
        for v in s.players.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        s
    }

    /// The season a day falls in.
    pub fn season_of(&self, day: i32) -> Option<i64> {
        self.list.iter().find(|x| day >= x.1 && day <= x.2).map(|x| x.0)
    }

    /// The roster a team is read at on `day`: this season's, or last season's
    /// while this one has none.
    pub fn current(&self, team: i64, day: i32) -> Option<&Vec<i64>> {
        let s = self.season_of(day)?;
        self.players.get(&(team, s)).or_else(|| self.players.get(&(team, s - 1)))
    }
    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let mut o = String::new();
        for (id, a, b) in &self.list {
            let _ = writeln!(o, "S\t{id}\t{}\t{}", show_day(*a), show_day(*b));
        }
        for (t, x) in &self.has {
            let _ = writeln!(o, "R\t{t}\t{x}");
        }
        for (pl, r) in &self.strength {
            let _ = writeln!(o, "Q\t{pl}\t{r:.0}");
        }
        for ((t, x), v) in &self.players {
            for pl in v {
                let _ = writeln!(o, "P\t{t}\t{x}\t{pl}");
            }
        }
        std::fs::write(path, o)
    }
    /// Whether the team is listed on `day`: a base roster for the season, or
    /// one last season while the new one is younger than `grace` days.
    pub fn listed(&self, team: i64, day: i32, k: &Knobs) -> bool {
        if k.grace < 0 {
            return true;
        }
        let Some(&(s, start, _)) = self.list.iter().find(|x| day >= x.1 && day <= x.2) else {
            return true;
        };
        self.has.contains(&(team, s)) || (day - start <= k.grace && self.has.contains(&(team, s - 1)))
    }
}

// --------------------------------------------------------------------- knobs

#[derive(Clone, Debug)]
pub struct Knobs {
    /// "strength_place": the score-based strength over place^place_pow;
    /// "score", "place" or "mix": `mix` of score and 1 - mix of place.
    pub perf: String,
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
    pub field_mode: String,
    /// Field strength is read over the top `field_k` lineups, empty seats at 0.
    pub field_k: usize,
    pub field_zero: f64,
    pub field_ref: f64,
    pub field_gamma: f64,
    pub field_min: f64,
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
    /// "linear" to zero at `zero_days`, "exp" halving every `half_life` days
    /// and dropped at `tail_days`, or "none": a result keeps its value forever.
    pub fade: String,
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
    pub overlap_by: String,
    /// Published points added to a sole first place before the weights.
    pub win_bonus: f64,
}

impl Default for Knobs {
    /// The variant on the site's page.
    fn default() -> Knobs {
        Knobs {
            perf: "strength_place".into(),
            place_pow: 0.5,
            mix: 0.5,
            shift: -800.0,
            w_sync: 0.75,
            w_strict: 0.75,
            w_async: 0.0,
            // Twenty lineups, with the anchors moved to where 5000 and 9500
            // on the top-10 mean fall on the top-20 one (S20 = 1.55 S10 - 5700).
            field_mode: "sum".into(),
            field_k: 20,
            field_zero: 2000.0,
            field_ref: 9000.0,
            field_gamma: 3.0,
            field_min: 0.0,
            sum_ref: 1_000_000.0,
            sum_gamma: 0.5,
            scale: 0.0,
            country_pow: 0.0,
            country_min: 1,
            town_pow: 0.0,
            town_min: 1,
            fade: "yearly".into(),
            full_days: 365,
            zero_days: 730,
            sync_full: 90,
            sync_zero: 365,
            half_life: 180.0,
            sync_half_life: 137.0,
            loss_ip: 0.1,
            loss_sync: 0.5,
            tail_days: 1826,
            slow_ref: 9000.0,
            slow_k: 1.0,
            top_x: 7,
            min_slots: 7,
            base_rule: true,
            grace: 60,
            active_days: 365,
            overlap_k: 1.0,
            overlap_by: "strength".into(),
            win_bonus: 0.0,
        }
    }
}

/// Every knob as the lab shows it: key, short label, what it does.
pub const KNOBS: &[(&str, &str, &str)] = &[
    ("top_x", "Результатов", "Сколько лучших результатов складывается в рейтинг"),
    ("min_slots", "Мин. слотов", "Делитель не меньше этого числа (пустой слот = 0); 0 — всегда делить на число результатов"),
    ("perf", "Игра по", "strength_place — показанная сила / место^степень; score — по взятым вопросам; place — по месту в поле; mix — смесь двух последних"),
    ("place_pow", "Степень места", "strength_place: показанная сила делится на место в этой степени (0.5 — корень)"),
    ("mix", "Доля взятых", "В смеси: доля оценки по взятым вопросам"),
    ("shift", "Сдвиг взятых", "Добавляется к оценке по взятым, чтобы она была на шкале составов"),
    ("w_sync", "Вес синхрона", "Множитель синхронного результата"),
    ("w_strict", "Вес строгого синхрона", "Множитель строго синхронного результата"),
    ("w_async", "Вес асинхрона", "Множитель асинхронного результата (0 — не считать)"),
    ("field_mode", "Поле: режим", "sum — очный: (сумма сил всех составов / опора)^степень, синхрон: как world; world — сумма сильнейших составов турнира / сумма 20 сильнейших баз мира на ту неделю (не больше 1); anchors — по порогам"),
    ("scale", "Масштаб", "Рейтинг = среднее «Сейчас» × масштаб; 0 — монотонная шкала a·x^p, подобранная так, чтобы у первой сотни рейтинг был близок к показанной силе"),
    ("sum_ref", "sum: опора", "sum: сумма сил всех составов очного турнира, при которой вес 1"),
    ("sum_gamma", "sum: степень", "sum: степень отношения суммы к опоре"),
    ("field_k", "Поле: топ-k", "Сила поля — средняя сила k сильнейших составов, недостающие считаются нулём"),
    ("field_zero", "Поле: вес 0 при", "Сила поля, при которой вес турнира 0"),
    ("field_ref", "Поле: вес 1 при", "Сила поля, при которой вес турнира 1"),
    ("field_gamma", "Поле: степень", "Степень кривой веса поля; 0 — все турниры с весом 1"),
    ("field_min", "Поле: мин. вес", "Нижняя граница веса поля"),
    ("country_pow", "Страны: степень", "Очный турнир: вес умножается на (число стран участников)^степень; 0 — не учитывать"),
    ("country_min", "Страны: от команд", "Страна считается, если от неё не меньше стольких команд"),
    ("town_pow", "Города: степень", "Очный турнир: вес умножается на (число городов участников)^степень; 0 — не учитывать"),
    ("town_min", "Города: от команд", "Город считается, если от него не меньше стольких команд"),
    ("fade", "Затухание", "yearly — теряет долю в год с первого дня; none — не затухает никогда; linear — линейно до нуля; exp — полураспад"),
    ("loss_ip", "yearly: очный теряет в год", "yearly: доля ценности очного результата, теряемая за год"),
    ("loss_sync", "yearly: синхрон теряет в год", "yearly: доля ценности синхронного результата, теряемая за год"),
    ("full_days", "Полная ценность, дней", "Сколько дней очный результат не затухает"),
    ("zero_days", "linear: ноль через", "linear: на какой день очный результат обнуляется"),
    ("half_life", "exp: полураспад, дней", "exp: за сколько дней очный результат теряет половину"),
    ("sync_full", "Синхрон: полная, дней", "Сколько дней синхронный результат не затухает"),
    ("sync_zero", "Синхрон linear: ноль", "linear: на какой день синхрон обнуляется; -1 — как очный"),
    ("sync_half_life", "Синхрон exp: полураспад", "exp: полураспад синхрона; 0 — как очный"),
    ("tail_days", "exp: отбросить после", "exp: результат старше стольких дней не считается"),
    ("slow_k", "Сильное медленнее, степень", "Возраст результата идёт со скоростью (опора / ценность)^k; 0 — одинаково для всех"),
    ("slow_ref", "Сильное медленнее, опора", "Ценность, при которой возраст идёт с обычной скоростью"),
    ("base_rule", "Только свой состав", "1 — результат идёт в зачёт, только если не меньше 4 базовых и не больше 2 легионеров"),
    ("grace", "Без базы: дней", "Сколько дней нового сезона команда без заявленной базы остаётся в таблице; -1 — всегда"),
    ("active_days", "Активность, дней", "Команда в таблице, только если за столько дней у неё есть зачтённый результат; 0 — без требования"),
    ("overlap_k", "Состав: степень", "Результат умножается на долю тогдашней базы, оставшейся в нынешней, в этой степени; 0 — не учитывать"),
    ("overlap_by", "Состав: по", "strength — по доле рейтинга игроков, count — по числу игроков (в обоих случаях шесть сильнейших)"),
    ("win_bonus", "Бонус за 1 место", "Очки, прибавляемые к игре за чистое первое место"),
];

impl Knobs {
    pub fn get(&self, key: &str) -> String {
        match key {
            "perf" => self.perf.clone(),
            "place_pow" => self.place_pow.to_string(),
            "field_mode" => self.field_mode.clone(),
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
            "country_pow" => self.country_pow.to_string(),
            "country_min" => self.country_min.to_string(),
            "town_pow" => self.town_pow.to_string(),
            "town_min" => self.town_min.to_string(),
            "fade" => self.fade.clone(),
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
            "base_rule" => (self.base_rule as u8).to_string(),
            "grace" => self.grace.to_string(),
            "active_days" => self.active_days.to_string(),
            "overlap_k" => self.overlap_k.to_string(),
            "overlap_by" => self.overlap_by.clone(),
            "win_bonus" => self.win_bonus.to_string(),
            _ => String::new(),
        }
    }

    pub fn set(&mut self, key: &str, val: &str) -> Result<(), String> {
        let val = val.trim();
        let num = || val.parse::<f64>().map_err(|_| format!("{key}: не число: {val}"));
        match key {
            "perf" if ["strength_place", "score", "place", "mix"].contains(&val) => self.perf = val.into(),
            "field_mode" if ["sum", "world", "anchors"].contains(&val) => self.field_mode = val.into(),
            "field_mode" => return Err(format!("{key}: нет такого значения: {val}")),
            "place_pow" => self.place_pow = num()?,
            "sum_ref" => self.sum_ref = num()?.max(1.0),
            "scale" => self.scale = num()?.max(0.0),
            "sum_gamma" => self.sum_gamma = num()?,
            "fade" if ["none", "linear", "exp", "yearly"].contains(&val) => self.fade = val.into(),
            "perf" | "fade" => return Err(format!("{key}: нет такого значения: {val}")),
            "mix" => self.mix = num()?.clamp(0.0, 1.0),
            "shift" => self.shift = num()?,
            "w_sync" => {
                self.w_sync = num()?;
                self.w_strict = self.w_sync;
            }
            "w_strict" => self.w_strict = num()?,
            "w_async" => self.w_async = num()?,
            "field_k" => self.field_k = (num()? as usize).clamp(1, 50),
            "field_zero" => self.field_zero = num()?,
            "field_ref" => self.field_ref = num()?,
            "field_gamma" => self.field_gamma = num()?,
            "field_min" => self.field_min = num()?,
            "country_pow" => self.country_pow = num()?,
            "country_min" => self.country_min = (num()? as usize).max(1),
            "town_pow" => self.town_pow = num()?,
            "town_min" => self.town_min = (num()? as usize).max(1),
            "full_days" => self.full_days = num()? as i32,
            "zero_days" => self.zero_days = num()? as i32,
            "sync_full" => self.sync_full = num()? as i32,
            "sync_zero" => self.sync_zero = num()? as i32,
            "half_life" => self.half_life = num()?,
            "loss_ip" => self.loss_ip = num()?.clamp(0.0, 1.0),
            "loss_sync" => self.loss_sync = num()?.clamp(0.0, 1.0),
            "sync_half_life" => self.sync_half_life = num()?,
            "tail_days" => self.tail_days = num()? as i32,
            "slow_ref" => self.slow_ref = num()?,
            "slow_k" => self.slow_k = num()?,
            "top_x" => self.top_x = (num()? as usize).clamp(1, 100),
            "min_slots" => self.min_slots = (num()? as usize).min(100),
            "base_rule" => self.base_rule = num()? != 0.0,
            "grace" => self.grace = num()? as i32,
            "active_days" => self.active_days = num()? as i32,
            "overlap_k" => self.overlap_k = num()?,
            "overlap_by" if ["strength", "count"].contains(&val) => self.overlap_by = val.into(),
            "overlap_by" => return Err(format!("{key}: нет такого значения: {val}")),
            "win_bonus" => self.win_bonus = num()?,
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

    /// The knobs that differ from the defaults, as `key=value` pairs.
    pub fn diff(&self) -> Vec<(String, String)> {
        let d = Knobs::default();
        KNOBS
            .iter()
            .filter(|(key, _, _)| self.get(key) != d.get(key))
            .map(|(key, _, _)| (key.to_string(), self.get(key)))
            .collect()
    }
}

// ----------------------------------------------------------------- aggregate

/// Each tournament as the weights see it.
pub struct Field {
    /// The strongest lineups, best first, at most 50.
    pub top_q: Vec<f64>,
    pub n_teams: usize,
    pub n_venues: usize,
    /// The tournaments merged into this field, by id.
    pub venues: Vec<i64>,
    /// The season's 20 strongest base rosters that week, summed.
    pub ref20: f64,
    /// Every lineup's strength, summed, negatives at 0.
    pub sum_all: f64,
    pub in_person: bool,
    /// Teams per country and per town, largest first.
    pub countries: Vec<usize>,
    pub towns: Vec<usize>,
}

/// The frozen values, the names and the rosters, loaded once.
pub struct Lab {
    /// Rated placed results only, the ones that can ever count.
    pub rows: Vec<Res>,
    pub fields: HashMap<i64, Field>,
    pub names: Names,
    pub seasons: Seasons,
    pub last: i32,
    /// The fair rule's weighted team NDCG, tune and test.
    pub fair: (f64, f64),
}

impl Lab {
    pub fn load(dir: &str) -> std::io::Result<Lab> {
        let rs = load_results(&format!("{dir}/results.tsv"))?;
        Ok(Lab::from_results(rs, Names::load(&format!("{dir}/names.tsv")), Seasons::load(&format!("{dir}/rosters.tsv"))))
    }

    pub fn from_results(rs: Vec<Res>, names: Names, seasons: Seasons) -> Lab {
        let mut by_t: HashMap<i64, (Vec<f64>, usize, HashSet<i64>, f64)> = HashMap::new();
        let mut geo: HashMap<i64, (bool, HashMap<i64, usize>, HashMap<i64, usize>)> = HashMap::new();
        for r in &rs {
            let g = geo.entry(r.tid).or_default();
            g.0 = r.format == 'i';
            if r.country >= 0 {
                *g.1.entry(r.country).or_default() += 1;
            }
            if r.town >= 0 {
                *g.2.entry(r.town).or_default() += 1;
            }
            let e = by_t.entry(r.tid).or_default();
            e.3 = r.ref20;
            if r.q.is_finite() {
                e.0.push(r.q);
            }
            e.1 += 1;
            e.2.insert(r.venue);
        }
        let fields = by_t
            .into_iter()
            .map(|(t, (mut q, n, v, ref20))| {
                let sum_all: f64 = q.iter().map(|x| x.max(0.0)).sum();
                q.sort_by(|a, b| b.total_cmp(a));
                q.truncate(50);
                let mut venues: Vec<i64> = v.into_iter().collect();
                venues.sort_unstable();
                let g = geo.remove(&t).unwrap_or_default();
                let counts = |m: HashMap<i64, usize>| {
                    let mut v: Vec<usize> = m.into_values().collect();
                    v.sort_by(|a, b| b.cmp(a));
                    v
                };
                (
                    t,
                    Field {
                        top_q: q,
                        n_teams: n,
                        n_venues: venues.len(),
                        venues,
                        ref20,
                        sum_all,
                        in_person: g.0,
                        countries: counts(g.1),
                        towns: counts(g.2),
                    },
                )
            })
            .collect();
        let last = rs.iter().map(|r| r.day).max().unwrap_or(0);
        let rows: Vec<Res> = rs.into_iter().filter(|r| r.rated && r.place > 0.0).collect();
        let fair = ndcg(&rows, |r| r.fair_mu);
        Lab { rows, fields, names, seasons, last, fair }
    }
}

/// One counted result: its value before fading, and what it came from.
#[derive(Clone, Copy)]
pub struct Val {
    pub day: i32,
    pub tid: i64,
    pub venue: i64,
    pub place: f64,
    pub vplace: f64,
    pub format: char,
    pub sync: bool,
    /// Peak value, performance and the weights behind it.
    pub base: f64,
    pub perf: f64,
    pub by_score: f64,
    pub by_place: f64,
    pub w: f64,
    pub wfmt: f64,
    pub fs: f64,
    pub nt: usize,
    pub nv: usize,
    /// The season it was played in.
    pub season: i64,
    /// The world's top 20 base rosters that week, summed.
    pub ref20: f64,
    pub sum_all: f64,
    /// The reach factor and the counts behind it.
    pub geo: f64,
    pub n_countries: usize,
    pub n_towns: usize,
}

pub struct Book {
    /// Per team, every counted result.
    pub vals: HashMap<i64, Vec<Val>>,
}

fn own_lineup(r: &Res) -> bool {
    r.n_base >= 4 && r.n - r.n_base <= 2
}

fn perf_of(r: &Res, k: &Knobs) -> (f64, f64, f64) {
    let (s, p) = (r.perf_s + k.shift, r.perf_p);
    let v = match k.perf.as_str() {
        "strength_place" if r.place > 0.0 => s / r.place.powf(k.place_pow),
        "place" => p,
        "mix" if s.is_finite() && p.is_finite() => k.mix * s + (1.0 - k.mix) * p,
        _ if s.is_finite() => s,
        _ => p,
    };
    (v, s, p)
}

/// The field weight and the field strength it was read from.
pub fn field_weight(f: &Field, k: &Knobs) -> (f64, f64) {
    let sum: f64 = f.top_q.iter().take(k.field_k).map(|x| x.max(0.0)).sum();
    let s = sum / k.field_k.max(1) as f64;
    if k.field_gamma == 0.0 {
        return (1.0, s);
    }
    if k.field_mode == "sum" && f.in_person {
        let w = (f.sum_all / k.sum_ref).powf(k.sum_gamma);
        return (w.max(k.field_min.min(1.0)), s);
    }
    let w = if k.field_mode == "world" || k.field_mode == "sum" {
        if f.ref20 > 0.0 {
            (sum / f.ref20).min(1.0).powf(k.field_gamma)
        } else {
            1.0
        }
    } else if k.field_ref <= k.field_zero {
        1.0
    } else {
        ((s - k.field_zero) / (k.field_ref - k.field_zero)).max(0.0).powf(k.field_gamma).min(1.0)
    };
    (w.max(k.field_min.min(1.0)), s)
}

/// The in-person reach factor and the country and town counts behind it.
pub fn reach(f: &Field, k: &Knobs) -> (f64, usize, usize) {
    let nc = f.countries.iter().filter(|&&c| c >= k.country_min).count();
    let nt = f.towns.iter().filter(|&&c| c >= k.town_min).count();
    if !f.in_person {
        return (1.0, nc, nt);
    }
    let x = (nc.max(1) as f64).powf(k.country_pow) * (nt.max(1) as f64).powf(k.town_pow);
    (x, nc, nt)
}

pub fn book(lab: &Lab, k: &Knobs) -> Book {
    let mut fw: HashMap<i64, (f64, f64)> = HashMap::new();
    let mut vals: HashMap<i64, Vec<Val>> = HashMap::new();
    for r in &lab.rows {
        if k.base_rule && !own_lineup(r) {
            continue;
        }
        let wfmt = match r.format {
            'i' => 1.0,
            's' => k.w_sync,
            'S' => k.w_strict,
            _ => k.w_async,
        };
        let (perf, by_score, by_place) = perf_of(r, k);
        if !perf.is_finite() || wfmt <= 0.0 {
            continue;
        }
        let Some(f) = lab.fields.get(&r.tid) else { continue };
        let (wf, fs) = *fw.entry(r.tid).or_insert_with(|| field_weight(f, k));
        let (geo, n_countries, n_towns) = reach(f, k);
        let w = wfmt * wf * geo;
        let base = w * (perf.max(0.0) + if r.place == 1.0 { k.win_bonus } else { 0.0 });
        vals.entry(r.team).or_default().push(Val {
            day: r.day,
            tid: r.tid,
            venue: r.venue,
            place: r.place,
            vplace: r.vplace,
            format: r.format,
            sync: r.format != 'i',
            base,
            perf,
            by_score,
            by_place,
            w,
            wfmt,
            fs,
            nt: f.n_teams,
            nv: f.n_venues,
            season: lab.seasons.season_of(r.day).unwrap_or(-1),
            ref20: f.ref20,
            sum_all: f.sum_all,
            geo,
            n_countries,
            n_towns,
        });
    }
    Book { vals }
}

pub fn fade(age: i32, v: &Val, k: &Knobs) -> f64 {
    if k.fade == "none" {
        return 1.0;
    }
    let days = age;
    let sync = v.sync && k.sync_zero >= 0;
    let (full, zero) = if sync {
        (k.sync_full as f64, k.sync_zero as f64)
    } else {
        (k.full_days as f64, k.zero_days as f64)
    };
    let age = if k.slow_k != 0.0 && v.base > 0.0 {
        age as f64 * (k.slow_ref / v.base).powf(k.slow_k)
    } else {
        age as f64
    };
    if k.fade == "yearly" {
        let loss = if v.sync { k.loss_sync } else { k.loss_ip };
        return (1.0 - loss).max(0.0).powf(age / 365.25);
    }
    if k.fade == "exp" {
        let hl = if v.sync && k.sync_half_life > 0.0 { k.sync_half_life } else { k.half_life };
        return if days >= k.tail_days {
            0.0
        } else if age <= full {
            1.0
        } else if hl <= 0.0 {
            0.0
        } else {
            0.5f64.powf((age - full) / hl)
        };
    }
    if age <= full {
        1.0
    } else if age >= zero {
        0.0
    } else {
        1.0 - (age - full) / (zero - full)
    }
}

/// The rating on the morning of `day`, and the results that make it, each
/// with its faded value.
/// The share of the base roster a result was earned with that is still on
/// the team's roster on `day`, by seats or by rating points over the six
/// strongest, to the power `overlap_k`. 1 when either roster
/// is unknown.
fn overlap(lab: &Lab, team: i64, season: i64, now: Option<&Vec<i64>>, k: &Knobs) -> f64 {
    if k.overlap_k == 0.0 {
        return 1.0;
    }
    let (Some(now), Some(then)) = (now, lab.seasons.players.get(&(team, season))) else { return 1.0 };
    if then.is_empty() {
        return 1.0;
    }
    // Six play at a time: both sides are read over their six strongest, so a
    // roster of eight that keeps its best six has lost nothing it needed.
    let weight = |p: &i64| {
        if k.overlap_by == "count" {
            1.0
        } else {
            lab.seasons.strength.get(p).copied().unwrap_or(0.0)
        }
    };
    let top6 = |v: &mut Vec<f64>| -> f64 {
        v.sort_by(|a, b| b.total_cmp(a));
        v.iter().take(6).sum()
    };
    let all = top6(&mut then.iter().map(weight).collect());
    let kept = top6(&mut then.iter().filter(|p| now.binary_search(p).is_ok()).map(weight).collect());
    if all <= 0.0 {
        return 1.0;
    }
    (kept / all).min(1.0).powf(k.overlap_k)
}

/// The rating on the morning of `day`, and the results that make it, each
/// with its value on that day: faded, and scaled by the roster overlap.
pub fn rating(lab: &Lab, b: &Book, team: i64, day: i32, k: &Knobs) -> (f64, Vec<(Val, f64)>) {
    let Some(v) = b.vals.get(&team) else { return (0.0, Vec::new()) };
    let now = if k.overlap_k != 0.0 { lab.seasons.current(team, day) } else { None };
    let mut ov: HashMap<i64, f64> = HashMap::new();
    let mut cur: Vec<(Val, f64)> = v
        .iter()
        .filter(|x| x.day < day)
        .map(|x| {
            let o = *ov.entry(x.season).or_insert_with(|| overlap(lab, team, x.season, now, k));
            (*x, x.base * fade(day - x.day, x, k) * o)
        })
        .filter(|x| x.1 > 0.0)
        .collect();
    cur.sort_by(|a, b| b.1.total_cmp(&a.1));
    cur.truncate(k.top_x);
    // A faded result holds only its remaining share of a slot, so an old
    // result near zero never drags the average down by taking a whole one.
    let held: f64 = cur.iter().map(|x| x.1 / x.0.base.max(1e-9)).sum();
    let slots = if k.min_slots == 0 {
        k.top_x as f64
    } else {
        held.clamp(k.min_slots.min(k.top_x) as f64, k.top_x as f64)
    };
    (cur.iter().map(|x| x.1).sum::<f64>() / slots, cur)
}

/// Every listed team with a positive rating on `day`, best first, published
/// through the display map `a·x^p`, which is returned beside them.
pub fn table(lab: &Lab, b: &Book, day: i32, k: &Knobs) -> (Vec<(i64, f64, Vec<(Val, f64)>)>, (f64, f64)) {
    let mut v: Vec<(i64, f64, Vec<(Val, f64)>)> = b
        .vals
        .keys()
        .filter(|&&t| lab.seasons.listed(t, day, k))
        .filter(|&&t| {
            k.active_days <= 0
                || b.vals[&t].iter().any(|x| x.day < day && day - x.day <= k.active_days)
        })
        .map(|&t| {
            let (x, c) = rating(lab, b, t, day, k);
            (t, x, c)
        })
        .filter(|x| x.1 > 0.0)
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    // The display map makes the numbers read as strengths: a·x^p, fitted in
    // logs over the top 100 so each team's number sits near the mean strength
    // its counted results demonstrated. It is monotone, so the order stands.
    let (a, p) = if k.scale > 0.0 {
        (k.scale, 1.0)
    } else {
        let pts: Vec<(f64, f64)> = v
            .iter()
            .take(100)
            .filter_map(|(_, x, c)| {
                let d: Vec<f64> = c.iter().map(|(val, _)| val.by_score).filter(|x| x.is_finite() && *x > 0.0).collect();
                (!d.is_empty() && *x > 0.0).then(|| (x.ln(), (d.iter().sum::<f64>() / d.len() as f64).ln()))
            })
            .collect();
        let n = pts.len() as f64;
        if n < 5.0 {
            (1.0, 1.0)
        } else {
            let (mx, my) = (pts.iter().map(|q| q.0).sum::<f64>() / n, pts.iter().map(|q| q.1).sum::<f64>() / n);
            let sxy: f64 = pts.iter().map(|q| (q.0 - mx) * (q.1 - my)).sum();
            let sxx: f64 = pts.iter().map(|q| (q.0 - mx) * (q.0 - mx)).sum();
            let p = if sxx > 0.0 { (sxy / sxx).clamp(0.05, 1.0) } else { 1.0 };
            ((my - p * mx).exp(), p)
        }
    };
    for x in v.iter_mut() {
        x.1 = a * x.1.powf(p);
    }
    let scale = (a, p);
    (v, scale)
}

// -------------------------------------------------------------------- metric

pub const TUNE: (&str, &str) = ("2018-09-01", "2024-08-29");
pub const TEST: (&str, &str) = ("2024-08-30", "2026-08-27");

fn tiekey(tid: i64, team: i64) -> u64 {
    let mut x = (tid as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(team as u64);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn avg_ranks(v: &[f64]) -> Vec<f64> {
    let mut ord: Vec<usize> = (0..v.len()).collect();
    ord.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut r = vec![0.0; v.len()];
    let mut i = 0;
    while i < ord.len() {
        let mut j = i;
        while j + 1 < ord.len() && v[ord[j + 1]] == v[ord[i]] {
            j += 1;
        }
        for &t in &ord[i..=j] {
            r[t] = (i + j) as f64 / 2.0 + 1.0;
        }
        i = j + 1;
    }
    r
}

/// NDCG of one field, the arithmetic of `team_ndcg::score`.
fn field_ndcg(tid: i64, team: &[i64], mu: &[f64], place: &[f64]) -> Option<f64> {
    if team.len() < 5 {
        return None;
    }
    let ranks = avg_ranks(place);
    let mut ord: Vec<usize> = (0..team.len()).collect();
    ord.sort_by(|&a, &b| mu[b].total_cmp(&mu[a]).then(tiekey(tid, team[a]).cmp(&tiekey(tid, team[b]))));
    let rel: Vec<f64> = ord.iter().map(|&i| 1.0 / ranks[i]).collect();
    let g = |r: &[f64]| r.iter().enumerate().map(|(i, r)| r / ((i + 2) as f64).log2()).sum::<f64>();
    let mut s = rel.clone();
    s.sort_by(|a, b| b.total_cmp(a));
    let idcg = g(&s);
    (idcg > 0.0).then(|| g(&rel) / idcg)
}

/// The team NDCG of DESIGN 7.3, weighted, over the tune and test windows.
pub fn ndcg(rows: &[Res], mut score: impl FnMut(&Res) -> f64) -> (f64, f64) {
    let win = [
        (parse_day(TUNE.0).unwrap(), parse_day(TUNE.1).unwrap()),
        (parse_day(TEST.0).unwrap(), parse_day(TEST.1).unwrap()),
    ];
    let mut by_t: HashMap<i64, (i32, f64, Vec<&Res>)> = HashMap::new();
    for r in rows.iter().filter(|r| r.format == 'i' && r.qual) {
        let e = by_t.entry(r.tid).or_insert((r.day, r.tw, Vec::new()));
        e.2.push(r);
    }
    let mut acc = [(0.0, 0.0); 2];
    for (tid, (day, tw, v)) in &by_t {
        let Some(w) = win.iter().position(|&(a, b)| *day >= a && *day < b) else { continue };
        let (mut team, mut mu, mut place) = (Vec::new(), Vec::new(), Vec::new());
        for r in v {
            let s = score(r);
            if s.is_finite() {
                team.push(r.team);
                mu.push(s);
                place.push(r.place);
            }
        }
        if let Some(x) = field_ndcg(*tid, &team, &mu, &place) {
            acc[w].0 += tw * x;
            acc[w].1 += tw;
        }
    }
    let m = |a: (f64, f64)| if a.1 > 0.0 { a.0 / a.1 } else { f64::NAN };
    (m(acc[0]), m(acc[1]))
}

/// The same metric for a set of knobs.
pub fn ndcg_of(lab: &Lab, b: &Book, k: &Knobs) -> (f64, f64) {
    ndcg(&lab.rows, |r| rating(lab, b, r.team, r.day, k).0)
}

// ---------------------------------------------------------------------- page

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const HELP: &str = " style=\"text-decoration: underline dotted; cursor: help\"";

/// Why a result's weight is below 1, for the hover text of its cell.
fn weight_title(v: &Val, k: &Knobs) -> String {
    if (v.w - 1.0).abs() < 0.005 && v.geo == 1.0 {
        return String::new();
    }
    if k.field_mode == "sum" && v.format == 'i' {
        let wf = v.w / v.geo.max(1e-9);
        let mut t = format!(
            "поле: сумма сил всех {} составов {:.0}, опора {:.0}, вес ({:.2})^{} = ×{:.2}",
            v.nt,
            v.sum_all,
            k.sum_ref,
            v.sum_all / k.sum_ref,
            k.sum_gamma,
            wf
        );
        if v.geo != 1.0 {
            let _ = write!(t, "; география ×{:.2}", v.geo);
        }
        return format!(" title=\"{}\"{HELP}", esc(&t));
    }
    let wf = v.w / (v.wfmt * v.geo).max(1e-9);
    let mut parts = Vec::new();
    if v.wfmt < 1.0 {
        let f = match v.format {
            'a' => "асинхрон",
            'S' => "строгий синхрон",
            _ => "синхрон",
        };
        parts.push(format!("{f}: ×{:.2}", v.wfmt));
    }
    if wf < 0.995 && k.field_mode == "world" {
        parts.push(format!(
            "поле: 20 сильнейших составов турнира вместе {:.0}, 20 сильнейших баз мира в ту неделю {:.0}, вес ×{:.2}",
            v.fs * k.field_k as f64,
            v.ref20,
            wf
        ));
    } else if wf < 0.995 {
        parts.push(format!(
            "поле: средняя сила {} сильнейших составов {:.0}, вес ×{:.2} (0 при {:.0}, 1 при {:.0})",
            k.field_k, v.fs, wf, k.field_zero, k.field_ref
        ));
    }
    if v.geo != 1.0 {
        let mut g = Vec::new();
        if k.country_pow != 0.0 {
            g.push(format!("стран {}", v.n_countries));
        }
        if k.town_pow != 0.0 {
            g.push(format!("городов {}", v.n_towns));
        }
        parts.push(format!("география: {}, ×{:.2}", g.join(", "), v.geo));
    }
    format!(" title=\"{}\"{HELP}", esc(&parts.join("; ")))
}

/// The rules in force, in English, for the lede.
pub fn describe(k: &Knobs, day: i32) -> String {
    let how = match k.perf.as_str() {
        "strength_place" => format!(
            "the strength it demonstrated divided by its place to the power {}: the demonstrated strength is \
             that of a lineup of six equal players who would expect to take as many questions on that package. \
             In a tournament played in several cities the place is in the combined standings, and a question \
             dropped at one venue is dropped at all of them.",
            k.place_pow
        ),
        "place" => "the strength at which a lineup would expect to finish where the team finished against the \
                    lineups of this field."
            .to_string(),
        "mix" => format!(
            "a blend of two strengths ({:.0}% and {:.0}%): the lineup of six equal players who would expect to \
             take as many questions on that package, and the one that would expect to finish where the team \
             finished against the lineups of this field. The first rewards the margin, the second a win in a \
             large strong field; hover over the number to see both. In a tournament played in several cities a \
             question dropped at one venue is dropped at all of them.",
            100.0 * k.mix,
            100.0 * (1.0 - k.mix)
        ),
        _ => "the strength of a lineup of six equal players who would expect to take as many questions on that \
              package."
            .to_string(),
    };
    let sync_full = if k.sync_zero >= 0 { k.sync_full } else { k.full_days };
    let fade = match k.fade.as_str() {
        "none" => "Results never fade.".to_string(),
        "yearly" => format!(
            "A result loses {:.0}% of its value per year if played in person and {:.0}% if synchronous, from \
             the day it is played.",
            100.0 * k.loss_ip,
            100.0 * k.loss_sync
        ),
        "exp" => format!(
            "An in-person result keeps its full value for {} days, then halves every {:.0} days; a synchronous \
             one keeps it for {} days, then halves every {:.0} days. Anything older than {:.1} years is dropped.",
            k.full_days,
            k.half_life,
            sync_full,
            if k.sync_half_life > 0.0 { k.sync_half_life } else { k.half_life },
            k.tail_days as f64 / 365.25
        ),
        _ => format!(
            "An in-person result keeps its full value for {} days and is worth nothing after {}; a synchronous \
             one keeps it for {} days and is worth nothing after {}.",
            k.full_days,
            k.zero_days,
            sync_full,
            if k.sync_zero >= 0 { k.sync_zero } else { k.zero_days }
        ),
    };
    let slow = if k.slow_k != 0.0 && k.fade != "none" {
        format!(
            " A strong result fades more slowly: its age runs at ({:.0} / peak value)<sup>{}</sup> of the calendar.",
            k.slow_ref, k.slow_k
        )
    } else {
        String::new()
    };
    let slots = if k.min_slots == 0 {
        format!("divided by {}; an empty slot counts as zero", k.top_x)
    } else {
        format!(
            "divided by the number of slots they fill, but never by fewer than {}; a faded result fills only its \
             faded share of a slot",
            k.min_slots.min(k.top_x)
        )
    };
    let active = if k.active_days > 0 {
        format!(" A team is listed only with a counted result in the last {} days.", k.active_days)
    } else {
        String::new()
    };
    let roster = if k.grace < 0 {
        "Every team is listed, whatever its roster.".to_string()
    } else {
        format!(
            "A team without a base roster for the season leaves the table {} days after the season starts.",
            k.grace
        )
    };
    format!(
        "On {date}. A team's rating is the sum of its {x} best current results, {slots}. Each result is worth \
         what the team played like, {how} The peak value is that times the format weight (in-person 1, \
         synchronous {ws}) and the field weight: {field}. {geo}{fade}{slow} {own}{roster}{active} Cities playing one package within two days of \
         each other are one tournament.",
        date = show_day(day),
        geo = {
            let mut g = Vec::new();
            if k.country_pow != 0.0 {
                g.push(format!(
                    "the number of countries its teams come from (a country counting from {} team{}) to the power {}",
                    k.country_min,
                    if k.country_min == 1 { "" } else { "s" },
                    k.country_pow
                ));
            }
            if k.town_pow != 0.0 {
                g.push(format!(
                    "the number of towns (from {} team{}) to the power {}",
                    k.town_min,
                    if k.town_min == 1 { "" } else { "s" },
                    k.town_pow
                ));
            }
            if g.is_empty() {
                String::new()
            } else {
                format!("An in-person result is further multiplied by {}. ", g.join(" and by "))
            }
        },
        x = k.top_x,
        ws = k.w_sync,
        field = if k.field_mode == "sum" {
            format!(
                "in person, (the summed strength of every lineup in the field / {:.0})<sup>{}</sup>, with no cap, \
                 so a field the size of the biggest festivals weighs about 1; synchronous, the combined strength of \
                 the {} strongest lineups over that of the season's 20 strongest base rosters that week, at most 1, \
                 to the power {}",
                k.sum_ref, k.sum_gamma, k.field_k, k.field_gamma
            )
        } else if k.field_mode == "world" {
            format!(
                "the combined strength of the tournament's {} strongest lineups over that of the season's 20 \
                 strongest base rosters that week, at most 1, to the power {}",
                k.field_k, k.field_gamma
            )
        } else {
            format!(
                "0 when the mean of the {} strongest lineups is {:.0} or less, 1 at {:.0} or more",
                k.field_k, k.field_zero, k.field_ref
            )
        },
        own = if k.base_rule {
            "Only results played by the team's own lineup count (at least four base-roster players, at most two \
             guests). "
        } else {
            ""
        },
    )
}

/// The table body: every team of the top with the results its rating is made
/// of, each at its peak and as it stands on `day`.
pub fn render_table(
    v: &[(i64, f64, Vec<(Val, f64)>)],
    k: &Knobs,
    day: i32,
    n: usize,
    was: &HashMap<i64, usize>,
    names: &Names,
    fields: &HashMap<i64, Field>,
) -> String {
    let mut o = String::new();
    o.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th>Команда / турнир</th><th class=\"num\">Дата</th>\
         <th class=\"num\" title=\"Место в турнире (в сводном зачёте, если площадок несколько)\">Место</th>\
         <th class=\"num\" title=\"Средняя сила сильнейших составов турнира\">Сила поля</th>\
         <th class=\"num\" title=\"Сила игры: как играли бы шесть равных игроков\">Игра</th>\
         <th class=\"num\" title=\"Вес формата × вес поля × география\">Вес</th>\
         <th class=\"num\" title=\"Ценность результата в день игры\">Игра × Вес</th>\
         <th class=\"num\" title=\"Доля ценности, оставшаяся после затухания\">Осталось</th>\
         <th class=\"num\" title=\"Доля тогдашней базы, оставшаяся в нынешней\">Состав</th>\
         <th class=\"num\" title=\"Ценность сегодня\">Сейчас</th></tr></thead><tbody>",
    );
    let fmtp = |x: f64| if x.fract() == 0.0 { format!("{x:.0}") } else { format!("{x:.1}") };
    for (i, (t, x, c)) in v.iter().take(n).enumerate() {
        let name = names.teams.get(t).cloned().unwrap_or_else(|| format!("Команда {t}"));
        let _ = write!(
            o,
            "<tr class=\"row-split\"><td class=\"num strong\">{}</td>\
             <td colspan=\"9\"><a href=\"/teams/{t}/\"><strong>{}</strong></a> \
             <span class=\"muted\">было {}</span></td><td class=\"num strong\">{:.0}</td></tr>",
            i + 1,
            esc(&name),
            was.get(t).map_or("—".into(), |r| r.to_string()),
            x
        );
        for (val, now) in c {
            let f = fade(day - val.day, val, k);
            let fmt = match val.format {
                'i' => "",
                'a' => " (асинхрон)",
                _ => " (синхрон)",
            };
            // A merged field is named by all its venues; the column holds the
            // merged place and the hover the place at the team's own venue.
            let venues = fields.get(&val.tid).map_or(vec![val.venue], |f| f.venues.clone());
            let title = venues
                .iter()
                .map(|id| {
                    format!(
                        "<a href=\"/tournaments/{id}/\">{}</a>",
                        esc(names.tours.get(id).map_or("?", |s| s.as_str()))
                    )
                })
                .collect::<Vec<_>>()
                .join(" / ");
            let (merged, place_title) = if val.nv > 1 {
                (
                    format!(" · сводный зачёт, команд: {}", val.nt),
                    format!(
                        " title=\"место на своей площадке ({}): {}\"{HELP}",
                        esc(names.tours.get(&val.venue).map_or("?", |s| s.as_str())),
                        fmtp(val.vplace)
                    ),
                )
            } else {
                (String::new(), String::new())
            };
            let perf_title = if k.perf == "strength_place" && val.by_score.is_finite() {
                format!(
                    " title=\"показанная сила {:.0} / место {}^{}\"{HELP}",
                    val.by_score,
                    fmtp(val.place),
                    k.place_pow
                )
            } else if k.perf == "mix" && val.by_score.is_finite() && val.by_place.is_finite() {
                format!(" title=\"по взятым вопросам {:.0}, по месту {:.0}\"{HELP}", val.by_score, val.by_place)
            } else {
                String::new()
            };
            let _ = write!(
                o,
                "<tr><td></td><td>{}<span class=\"muted\">{}{}</span></td>\
                 <td class=\"num muted\">{}</td><td class=\"num\"{}>{}</td>\
                 <td class=\"num muted\" title=\"команд: {}\">{:.0}</td><td class=\"num\"{}>{:.0}</td>\
                 <td class=\"num muted\"{}>{:.2}</td><td class=\"num\">{:.0}</td>\
                 <td class=\"num muted\">{:.0}%</td><td class=\"num muted\">{:.0}%</td><td class=\"num\">{:.0}</td></tr>",
                title,
                fmt,
                merged,
                show_day(val.day),
                place_title,
                fmtp(val.place),
                val.nt,
                val.fs,
                perf_title,
                val.perf,
                weight_title(val, k),
                val.w,
                val.base,
                100.0 * f,
                100.0 * now / (val.base * f).max(1e-9),
                now
            );
        }
        let held: f64 = c.iter().map(|(val, now)| now / val.base.max(1e-9)).sum();
        let empty = if k.min_slots == 0 { k.top_x } else { k.min_slots.min(k.top_x) };
        for _ in c.len()..empty.max((held.ceil() as usize).min(k.top_x)) {
            o.push_str(
                "<tr><td></td><td class=\"muted\">пустой слот</td><td></td><td></td><td></td><td></td><td></td>\
                 <td></td><td></td><td></td><td class=\"num muted\">0</td></tr>",
            );
        }
    }
    o.push_str("</tbody></table></div>");
    o
}

/// The whole lab page body: the form, the metric and the table.
pub fn render_lab(lab: &Lab, k: &Knobs, day: i32, n: usize, was: &HashMap<i64, usize>, err: &[String]) -> String {
    let t0 = std::time::Instant::now();
    let b = book(lab, k);
    let (v, scale) = table(lab, &b, day, k);
    let (tune, test) = ndcg_of(lab, &b, k);
    let ms = t0.elapsed().as_millis();
    let mut o = String::new();
    let _ = write!(
        o,
        "<header class=\"page-head\"><h1>Рейтинг достижений — лаборатория</h1>\
         <p class=\"lede\">{}</p></header>",
        describe(k, day)
    );
    o.push_str("<section class=\"section\"><form method=\"get\" action=\"/achieve/\" id=\"lab\">");
    o.push_str(
        "<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(12rem,1fr));gap:.5rem 1rem;\
         margin-bottom:1rem\">",
    );
    let d = Knobs::default();
    for (key, label, help) in KNOBS {
        let cur = k.get(key);
        let changed = cur != d.get(key);
        let input = match *key {
            "perf" | "fade" | "overlap_by" | "field_mode" => {
                let opts: &[&str] = match *key {
                    "perf" => &["strength_place", "mix", "score", "place"],
                    "field_mode" => &["sum", "world", "anchors"],
                    "fade" => &["yearly", "exp", "linear", "none"],
                    _ => &["strength", "count"],
                };
                let mut s = format!("<select name=\"{key}\">");
                for o in opts {
                    let _ = write!(s, "<option{}>{o}</option>", if *o == cur { " selected" } else { "" });
                }
                s.push_str("</select>");
                s
            }
            _ => format!(
                "<input name=\"{key}\" value=\"{}\" inputmode=\"decimal\" style=\"width:100%\">",
                esc(&cur)
            ),
        };
        let _ = write!(
            o,
            "<label title=\"{}\" style=\"display:flex;flex-direction:column;font-size:.85rem{}\">\
             <span>{}</span>{input}</label>",
            esc(help),
            if changed { ";font-weight:600" } else { "" },
            esc(label)
        );
    }
    let _ = write!(
        o,
        "<label style=\"display:flex;flex-direction:column;font-size:.85rem\"><span>На дату</span>\
         <input name=\"at\" value=\"{}\" style=\"width:100%\"></label>\
         <label style=\"display:flex;flex-direction:column;font-size:.85rem\"><span>Команд в таблице</span>\
         <input name=\"n\" value=\"{n}\" inputmode=\"numeric\" style=\"width:100%\"></label>",
        show_day(day)
    );
    o.push_str("</div><p><button type=\"submit\">Пересчитать</button> <a href=\"/achieve/\">Сбросить</a>");
    // Presets are links, so a result is shareable by its URL.
    let presets: &[(&str, &str)] = &[
        ("10 результатов", "top_x=10&min_slots=10"),
        ("без затухания", "fade=none"),
        ("без затухания, только активные за год", "fade=none&active_days=365"),
        ("смесь взятых и места", "perf=mix"),
        ("только по месту", "perf=place"),
        ("только по взятым", "perf=score"),
        ("синхрон как очный", "w_sync=1&sync_zero=-1&sync_half_life=0"),
        ("все турниры с весом 1", "field_gamma=0"),
        ("без правила базы", "base_rule=0&grace=-1"),
    ];
    o.push_str(" · Попробовать: ");
    for (i, (label, q)) in presets.iter().enumerate() {
        let _ = write!(o, "{}<a href=\"/achieve/?{q}\">{label}</a>", if i > 0 { " · " } else { "" });
    }
    o.push_str("</p></form>");
    for e in err {
        let _ = write!(o, "<p class=\"muted\">Не понял: {}</p>", esc(e));
    }
    let _ = write!(
        o,
        "<p class=\"muted\">Команд в таблице: {}. Предсказательная сила (NDCG порядка команд с базовым составом \
         на очных турнирах): обучение {:.4}, проверка {:.4}; у нынешнего рейтинга сайта {:.4} и {:.4}. \
         Рейтинг = {a:.1} × (среднее значений «Сейчас»)<sup>{p:.2}</sup> — монотонная шкала, подобранная так, чтобы у первой сотни рейтинг был близок к силе, показанной в зачтённых результатах; порядок она не меняет. Пересчёт занял {ms} мс. Жирным отмечены ручки, изменённые относительно умолчаний.</p>",
        v.len(),
        tune,
        test,
        lab.fair.0,
        lab.fair.1,
        a = scale.0,
        p = scale.1
    );
    o.push_str(&render_table(&v, k, day, n, was, &lab.names, &lab.fields));
    o.push_str("</section>");
    o.push_str(
        "<script>document.querySelectorAll('#lab select').forEach(function(s){\
         s.addEventListener('change',function(){s.form.submit()})});</script>",
    );
    o
}
