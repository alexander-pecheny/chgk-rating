//! How well each system forecast every tournament of the clean window: ours
//! and our reimplementations of rating.chgk.fun, chgk.gg, chgk.quest and elo-chgk.uk, each scored with the NDCG of
//! `src/ndcg.rs`. No rule of the model was chosen on a tournament after
//! `display.clean_from`, so these are honest out-of-sample numbers.

use crate::data::each;
use crate::index::Ctx;
use chgk_rating::ndcg::score_field;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::BTreeMap;
use std::path::Path;

/// Where a system's forecasts are read from.
enum Source {
    /// Our `out/ratings.db`.
    Ratings,
    /// A reimplementation's database under the ext dir.
    Ext(&'static str),
    /// One that forecasts by its `x` column, as `ext_elo` does.
    Pack(&'static str),
}

const SYSTEMS: &[(&str, Source)] =
    &[("ours", Source::Ratings), ("fun", Source::Ext("fun.db")), ("gg", Source::Ext("gg.db")), ("quest", Source::Ext("quest.db")), ("elo", Source::Pack("elo.db"))];

/// The `(tournament_id, f, actual_place)` rows of a source from `{from}` on,
/// `f` higher for a better forecast.
fn sql(src: &Source, from: &str) -> String {
    match src {
        Source::Ratings => format!(
            "select tournament_id, forecast as f, actual_place from team_events
             where date >= '{from}' and actual_place > 0 and forecast is not null"
        ),
        Source::Ext(_) => format!(
            "select tournament_id, -forecast_place as f, actual_place from team_events
             where date >= '{from}' and actual_place > 0 and forecast_place > 0"
        ),
        Source::Pack(_) => format!(
            "select tournament_id, x as f, actual_place from team_events
             where date >= '{from}' and actual_place > 0 and x is not null"
        ),
    }
}

pub fn build(c: &Ctx, ext: &Path, from: &str) {
    if from.is_empty() {
        return;
    }
    let mut ins = c.w.prepare("insert into forecast_ndcg values (?1, ?2, ?3, ?4, ?5)").unwrap();
    for (system, src) in SYSTEMS {
        let own;
        let db = match src {
            Source::Ratings => c.r,
            Source::Ext(f) | Source::Pack(f) => match Connection::open_with_flags(ext.join(f), OpenFlags::SQLITE_OPEN_READ_ONLY) {
                Ok(conn) => {
                    own = conn;
                    &own
                }
                Err(_) => continue,
            },
        };
        let mut n = 0;
        for (tid, field) in fields(db, &sql(src, from)) {
            if let Some(s) = score_field(tid, &field) {
                ins.execute(params![tid, system, s.ndcg, s.ndcg3, field.len() as i64]).unwrap();
                n += 1;
            }
        }
        crate::data::log(c.t0, &format!("forecast NDCG: {n} tournaments for {system}"));
    }
}

/// Each tournament's `(forecast, actual place)` pairs.
fn fields(db: &Connection, sql: &str) -> BTreeMap<i64, Vec<(f64, f64)>> {
    let mut out: BTreeMap<i64, Vec<(f64, f64)>> = BTreeMap::new();
    each(db, sql, |x| {
        let tid: i64 = x.get("tournament_id").unwrap();
        let (f, place): (f64, f64) = (x.get("f").unwrap(), x.get("actual_place").unwrap());
        out.entry(tid).or_default().push((f, place));
    });
    out
}
