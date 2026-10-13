//! Which tournaments are young fields: school, student or novice, the age
//! class of `tools/tournament_class.py`, worked out from buff.db alone so the
//! daily fit needs no table from a research run.
//!
//! An in-person or synchronous tournament from 2015 with at least five results
//! is young when its name is a school's or a student's, unless it is a
//! synchronous one whose median player started five or more years ago (adults
//! on a youth pack), or when 60% of its players started less than three years
//! ago. A player starts at their first game of any format.
use crate::data::Format;
use crate::util::parse_date;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

const FROM: &str = "2015-01-01";
const MIN_RESULTS: usize = 5;
const ADULT_YEARS: f64 = 5.0;
const NEW_YEARS: f64 = 3.0;
const NOVICE_SHARE: f64 = 0.6;
const DAYS_PER_YEAR: f64 = 365.25;
const SCHOOL: [&str; 4] = ["школ", "шчр", "детск", "юниор"];
const STUDENT: [&str; 5] = ["студ", "счр", "мчр", "вуз", "межфакультет"];

/// A field's name, format, day and every result's roster.
type Field = (String, Format, i32, Vec<Vec<i64>>);

/// U12, U-17 and the like: `u`, an optional hyphen, `1` and one digit, then
/// the end of the word.
fn under_age(name: &str) -> bool {
    let c: Vec<char> = name.chars().collect();
    (0..c.len()).any(|i| {
        let j = i + 1 + (c.get(i + 1) == Some(&'-')) as usize;
        c[i] == 'u'
            && c.get(j) == Some(&'1')
            && c.get(j + 1).is_some_and(|d| d.is_ascii_digit())
            && c.get(j + 2).is_none_or(|&e| !(e.is_alphanumeric() || e == '_'))
    })
}

fn youth_name(name: &str) -> bool {
    let n = name.to_lowercase();
    SCHOOL.iter().chain(&STUDENT).any(|s| n.contains(s)) || under_age(&n)
}

/// Rounded as the classifier's table prints them, so the cuts fall where they did.
fn round(x: f64, places: i32) -> f64 {
    let s = 10f64.powi(places);
    (x * s).round() / s
}

/// The ids of every young field in `conn`.
pub fn young_fields(conn: &Connection) -> rusqlite::Result<HashSet<i64>> {
    let mut st = conn.prepare(
        "SELECT t.id, t.name, t.tournament_type, substr(t.date_start,1,10), r.team_members
         FROM tournament_results r JOIN tournaments t ON t.id = r.id WHERE t.date_start >= '2000'",
    )?;
    let mut first: HashMap<i64, i32> = HashMap::new();
    let mut fields: HashMap<i64, Field> = HashMap::new();
    let mut rows = st.query([])?;
    while let Some(row) = rows.next()? {
        let kind: Option<String> = row.get(2)?;
        let date: Option<String> = row.get(3)?;
        let (Some(format), Some(day)) = (kind.as_deref().and_then(Format::parse), date.as_deref().and_then(parse_date)) else {
            continue;
        };
        let members: Vec<i64> = row.get::<_, Option<String>>(4)?.and_then(|m| serde_json::from_str(&m).ok()).unwrap_or_default();
        for &p in &members {
            first.entry(p).and_modify(|d| *d = (*d).min(day)).or_insert(day);
        }
        if date.as_deref() >= Some(FROM) {
            let name: Option<String> = row.get(1)?;
            fields.entry(row.get(0)?).or_insert_with(|| (name.unwrap_or_default(), format, day, Vec::new())).3.push(members);
        }
    }
    Ok(fields
        .into_iter()
        .filter(|(_, (name, format, day, results))| {
            if *format == Format::Async || results.len() < MIN_RESULTS {
                return false;
            }
            let mut years: Vec<f64> = results.iter().flatten().map(|p| (day - first[p]) as f64 / DAYS_PER_YEAR).collect();
            if years.is_empty() {
                return false;
            }
            years.sort_by(f64::total_cmp);
            let median = round(years[years.len() / 2], 2);
            let new = round(years.iter().filter(|&&y| y < NEW_YEARS).count() as f64 / years.len() as f64, 3);
            let adults = *format != Format::InPerson && median >= ADULT_YEARS;
            (youth_name(name) && !adults) || new >= NOVICE_SHARE
        })
        .map(|(id, _)| id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(youth_name("Школьный кубок"));
        assert!(youth_name("СтудЧР-2025"));
        assert!(youth_name("Кубок U-17"));
        assert!(youth_name("Кубок U12."));
        assert!(!youth_name("Кубок U123"));
        assert!(!youth_name("Чемпионат России"));
    }
}
