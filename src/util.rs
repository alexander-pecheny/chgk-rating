pub fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i32 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn parse_date(s: &str) -> Option<i32> {
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
    Some(days_from_civil(
        num(0, 4)?,
        num(5, 7)? as u32,
        num(8, 10)? as u32,
    ))
}

pub fn format_date(day: i32) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Today in UTC as `YYYY-MM-DD`, from the system clock.
pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_date((secs / 86_400) as i32)
}

/// 1970-01-01 was a Thursday, so Thursdays are exactly the days divisible by 7.
pub fn next_thursday(day: i32) -> i32 {
    day + ((7 - day.rem_euclid(7)) % 7)
}

/// The last Thursday on or before `day`, which is the last release of a window
/// that ends there.
pub fn last_thursday(day: i32) -> i32 {
    day - day.rem_euclid(7)
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn bit(&mut self) -> u8 {
        (self.next_u64() >> 33) as u8 & 1
    }
    pub fn normal(&mut self) -> f64 {
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        let v = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        (-2.0 * (u + 1e-12).ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// Spearman correlation, tie-corrected. None when either side is constant.
pub fn spearman(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() < 3 || a.len() != b.len() {
        return None;
    }
    let (ra, rb) = (avg_ranks(a), avg_ranks(b));
    let n = a.len() as f64;
    let (ma, mb) = (ra.iter().sum::<f64>() / n, rb.iter().sum::<f64>() / n);
    let mut num = 0.0;
    let (mut da, mut db) = (0.0, 0.0);
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
    let mut out = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (mut row, mut cell, mut quoted) = (Vec::new(), String::new(), false);
        for c in line.chars() {
            match c {
                '"' => quoted = !quoted,
                ',' if !quoted => row.push(std::mem::take(&mut cell)),
                _ => cell.push(c),
            }
        }
        row.push(cell);
        out.push(row);
    }
    if !out.is_empty() {
        out.remove(0);
    }
    Ok(out)
}

/// Average ranks of `key` ascending, ties sharing the mean of their places.
pub fn avg_ranks(key: &[f64]) -> Vec<f64> {
    let mut o: Vec<usize> = (0..key.len()).collect();
    o.sort_by(|&a, &b| key[a].total_cmp(&key[b]));
    let mut r = vec![0.0; key.len()];
    let mut i = 0;
    while i < o.len() {
        let mut j = i;
        while j + 1 < o.len() && key[o[j + 1]] == key[o[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &k in &o[i..=j] {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}
