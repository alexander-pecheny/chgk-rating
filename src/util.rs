//! Dates as days since 1970-01-01, a seeded random stream, ranks and a CSV
//! reader: the small things every module needs and none owns.

// The civil-calendar arithmetic of Howard Hinnant's `days_from_civil`, in
// 400-year eras that start on 1 March, so the leap day ends the year.
const DAYS_PER_ERA: i32 = 146_097;
const YEARS_PER_ERA: i32 = 400;
/// Days from 0000-03-01 to 1970-01-01.
const EPOCH_SHIFT: i32 = 719_468;
const DAYS_PER_YEAR: i32 = 365;
/// Leap-year corrections inside an era: every 4th year, but not every 100th.
const LEAP_EVERY: i32 = 4;
const NO_LEAP_EVERY: i32 = 100;
/// Day-of-era corrections that turn a day count back into a year of the era.
const DAYS_PER_4Y_LESS_1: i32 = 1460;
const DAYS_PER_100Y_LESS_1: i32 = 36_524;
/// Months from March: the day of year of month `mp` is `(153 mp + 2) / 5`.
const MONTH_SPAN: i32 = 153;
const MONTH_ROUND: i32 = 2;
const MONTH_PARTS: i32 = 5;
const MARCH: i32 = 3;
const MONTHS: i32 = 12;
/// `m + 9` mod 12 counts months from March.
const MARCH_OFFSET: i32 = 9;
const FEBRUARY: u32 = 2;
/// Counting months from March as 0, January is 10.
const JANUARY_FROM_MARCH: i32 = 10;

const SECONDS_PER_DAY: u64 = 86_400;
const DAYS_PER_WEEK: i32 = 7;

/// The Weyl increment and the two multipliers of splitmix64.
const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const MIX_A: u64 = 0xBF58_476D_1CE4_E5B9;
const MIX_B: u64 = 0x94D0_49BB_1331_11EB;
const SHIFT_A: u32 = 30;
const SHIFT_B: u32 = 27;
const SHIFT_C: u32 = 31;
/// The 53 bits of an f64 mantissa, taken from the top of a 64-bit draw.
const MANTISSA_SHIFT: u32 = 11;
const MANTISSA: f64 = (1u64 << 53) as f64;
const BIT_SHIFT: u32 = 33;
/// Keeps `ln u` finite in the Box-Muller draw.
const LN_GUARD: f64 = 1e-12;
const BOX_MULLER: f64 = -2.0;

/// Spearman needs three pairs before a correlation means anything.
const MIN_SPEARMAN: usize = 3;

/// `YYYY-MM-DD` at fixed offsets.
const YEAR_DIGITS: (usize, usize) = (0, 4);
const MONTH_DIGITS: (usize, usize) = (5, 7);
const DAY_DIGITS: (usize, usize) = (8, 10);
const DECIMAL: i32 = 10;

pub fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= FEBRUARY { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - (YEARS_PER_ERA - 1) } / YEARS_PER_ERA;
    let yoe = y - era * YEARS_PER_ERA;
    let mp = (m as i32 + MARCH_OFFSET) % MONTHS;
    let doy = (MONTH_SPAN * mp + MONTH_ROUND) / MONTH_PARTS + d as i32 - 1;
    let doe = yoe * DAYS_PER_YEAR + yoe / LEAP_EVERY - yoe / NO_LEAP_EVERY + doy;
    era * DAYS_PER_ERA + doe - EPOCH_SHIFT
}

pub fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z + EPOCH_SHIFT;
    let era = if z >= 0 { z } else { z - (DAYS_PER_ERA - 1) } / DAYS_PER_ERA;
    let doe = z - era * DAYS_PER_ERA;
    let yoe = (doe - doe / DAYS_PER_4Y_LESS_1 + doe / DAYS_PER_100Y_LESS_1 - doe / (DAYS_PER_ERA - 1)) / DAYS_PER_YEAR;
    let y = yoe + era * YEARS_PER_ERA;
    let doy = doe - (DAYS_PER_YEAR * yoe + yoe / LEAP_EVERY - yoe / NO_LEAP_EVERY);
    let mp = (MONTH_PARTS * doy + MONTH_ROUND) / MONTH_SPAN;
    let d = (doy - (MONTH_SPAN * mp + MONTH_ROUND) / MONTH_PARTS + 1) as u32;
    let m = if mp < JANUARY_FROM_MARCH { mp + MARCH } else { mp - MARCH_OFFSET } as u32;
    (if m <= FEBRUARY { y + 1 } else { y }, m, d)
}

/// The day of a `YYYY-MM-DD` date; anything after the tenth byte is ignored.
pub fn parse_date(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    if b.len() < DAY_DIGITS.1 {
        return None;
    }
    let num = |(a, z): (usize, usize)| -> Option<i32> {
        b[a..z].iter().try_fold(0i32, |v, &c| c.is_ascii_digit().then(|| v * DECIMAL + (c - b'0') as i32))
    };
    Some(days_from_civil(num(YEAR_DIGITS)?, num(MONTH_DIGITS)? as u32, num(DAY_DIGITS)? as u32))
}

pub fn format_date(day: i32) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The UTC day of the last complete update of the mirror at `db`: the start
/// time the updater stamps into `db_updates` when it finishes, such as
/// `2026-10-10T04:05:02+0300`. A run reads this rather than the clock, so the
/// same `buff.db` gives the same output whatever day it is run on.
pub fn data_day(db: &str) -> Option<String> {
    let conn = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let stamps: Vec<String> = conn
        .prepare("SELECT datetime FROM db_updates")
        .ok()?
        .query_map([], |r| r.get(0))
        .ok()?
        .filter_map(|x| x.ok())
        .collect();
    stamps.iter().filter_map(|s| utc_minutes(s)).max().map(|m| format_date(m.div_euclid(MINUTES_PER_DAY) as i32))
}

const MINUTES_PER_DAY: i64 = 24 * 60;

/// Minutes since 1970 in UTC of `YYYY-MM-DDTHH:MM:SS±HHMM`.
fn utc_minutes(s: &str) -> Option<i64> {
    let day = parse_date(s.get(..10)?)? as i64;
    let hm = |a: usize| s.get(a..a + 2)?.parse::<i64>().ok();
    let local = day * MINUTES_PER_DAY + hm(11)? * 60 + hm(14)?;
    let sign = match s.get(19..20) {
        Some("+") => 1,
        Some("-") => -1,
        _ => return Some(local),
    };
    Some(local - sign * (hm(20)? * 60 + hm(22)?))
}

/// Today in UTC as `YYYY-MM-DD`, from the system clock.
pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format_date((secs / SECONDS_PER_DAY) as i32)
}

/// 1970-01-01 was a Thursday, so Thursdays are exactly the days divisible by 7.
pub fn next_thursday(day: i32) -> i32 {
    day + ((DAYS_PER_WEEK - day.rem_euclid(DAYS_PER_WEEK)) % DAYS_PER_WEEK)
}

/// The last Thursday on or before `day`, which is the last release of a window
/// that ends there.
pub fn last_thursday(day: i32) -> i32 {
    day - day.rem_euclid(DAYS_PER_WEEK)
}

/// The splitmix64 finaliser: a bijection of `u64` that scatters nearby inputs.
pub fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> SHIFT_A)).wrapping_mul(MIX_A);
    z = (z ^ (z >> SHIFT_B)).wrapping_mul(MIX_B);
    z ^ (z >> SHIFT_C)
}

/// One number from a pair, the same on every run: `mix64(a·γ + b)`.
pub fn mix_pair(a: u64, b: u64) -> u64 {
    mix64(a.wrapping_mul(GOLDEN_GAMMA).wrapping_add(b))
}

/// Two `u32` as one `u64` key, `a` in the high half.
pub fn pack(a: u32, b: u32) -> u64 {
    ((a as u64) << u32::BITS) | b as u64
}

/// splitmix64, seeded.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(GOLDEN_GAMMA) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        mix64(self.0)
    }
    pub fn bit(&mut self) -> u8 {
        (self.next_u64() >> BIT_SHIFT) as u8 & 1
    }
    /// Uniform on `[0, 1)`.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> MANTISSA_SHIFT) as f64 / MANTISSA
    }
    /// Standard normal, by Box-Muller.
    pub fn normal(&mut self) -> f64 {
        let u = self.uniform();
        let v = self.uniform();
        (BOX_MULLER * (u + LN_GUARD).ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// Spearman correlation, tie-corrected. None when either side is constant.
pub fn spearman(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() < MIN_SPEARMAN || a.len() != b.len() {
        return None;
    }
    let (ra, rb) = (avg_ranks(a), avg_ranks(b));
    let n = a.len() as f64;
    let (ma, mb) = (ra.iter().sum::<f64>() / n, rb.iter().sum::<f64>() / n);
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for (x, y) in ra.iter().zip(&rb) {
        num += (x - ma) * (y - mb);
        da += (x - ma) * (x - ma);
        db += (y - mb) * (y - mb);
    }
    (da > 0.0 && db > 0.0).then(|| num / (da * db).sqrt())
}

/// Minimal CSV reader: strips a BOM, honours double quotes, drops the header.
pub fn csv_rows(path: &str) -> std::io::Result<Vec<Vec<String>>> {
    let text = std::fs::read_to_string(path)?;
    let lines = text.trim_start_matches('\u{feff}').lines().map(|l| l.trim_end_matches('\r'));
    Ok(lines.filter(|l| !l.is_empty()).skip(1).map(csv_line).collect())
}

fn csv_line(line: &str) -> Vec<String> {
    let (mut row, mut cell, mut quoted) = (Vec::new(), String::new(), false);
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut cell)),
            _ => cell.push(c),
        }
    }
    row.push(cell);
    row
}

/// Average ranks of `key` ascending, ties sharing the mean of their places.
pub fn avg_ranks(key: &[f64]) -> Vec<f64> {
    let mut o: Vec<usize> = (0..key.len()).collect();
    o.sort_by(|&a, &b| key[a].total_cmp(&key[b]));
    let mut r = vec![0.0; key.len()];
    for (i, j) in tie_runs(&o, |a, b| key[a] == key[b]) {
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &k in &o[i..=j] {
            r[k] = avg;
        }
    }
    r
}

/// The runs `(first, last)` of consecutive positions of `order` whose items are
/// `same`, in order.
pub fn tie_runs(order: &[usize], same: impl Fn(usize, usize) -> bool) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && same(order[j + 1], order[i]) {
            j += 1;
        }
        runs.push((i, j));
        i = j + 1;
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        for day in -80_000..80_000 {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day, "{y}-{m}-{d}");
            assert_eq!(parse_date(&format_date(day)), Some(day));
        }
    }

    #[test]
    fn update_stamps_in_utc() {
        let day = |s: &str| utc_minutes(s).map(|m| format_date(m.div_euclid(MINUTES_PER_DAY) as i32));
        assert_eq!(day("2026-10-10T04:05:02+0300").as_deref(), Some("2026-10-10"));
        assert_eq!(day("2026-10-10T02:05:02+0300").as_deref(), Some("2026-10-09"));
        assert_eq!(day("2026-10-10T22:30:00-0300").as_deref(), Some("2026-10-11"));
        assert_eq!(day("2026-10-10T22:30:00").as_deref(), Some("2026-10-10"));
        assert_eq!(day("garbage"), None);
    }

    #[test]
    fn known_dates() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(17_137), "2016-12-02");
        assert_eq!(format_date(11_016), "2000-02-29");
        assert_eq!(next_thursday(0), 0);
        assert_eq!(next_thursday(1), 7);
    }

    #[test]
    fn ranks_share_ties() {
        assert_eq!(avg_ranks(&[3.0, 1.0, 3.0, 2.0]), vec![3.5, 1.0, 3.5, 2.0]);
    }
}
