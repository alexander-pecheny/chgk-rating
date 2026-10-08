//! Builds the serving index, `out/web.db`, that the site server in `web/`
//! attaches beside `out/ratings.db`. Everything here is computation: names and
//! rosters lifted out of `buff.db`, the two ranking tables, the place history
//! behind each chart and the anomaly scores. The server only reads the result;
//! `schema.rs` is the contract between the two.
//!
//! ```text
//! webindex [--ratings out/ratings.db] [--buff buff.db] [--config config.toml]
//!          [--out out/web.db] [--rebuild]
//! ```
//!
//! The index is rebuilt only when it is missing, was built by another
//! `schema::VERSION`, or is older than either input; `--rebuild` forces it.

mod anomaly;
mod data;
mod history;
mod index;
mod results;
mod schema;

use std::path::{Path, PathBuf};

struct Args {
    ratings: PathBuf,
    buff: PathBuf,
    config: PathBuf,
    out: PathBuf,
    rebuild: bool,
}

fn args() -> Args {
    let mut a = std::env::args().skip(1);
    let mut v = Args {
        ratings: "out/ratings.db".into(),
        buff: "buff.db".into(),
        config: "config.toml".into(),
        out: "out/web.db".into(),
        rebuild: false,
    };
    while let Some(x) = a.next() {
        let mut next = || a.next().unwrap_or_else(|| panic!("{x} needs a value"));
        match x.as_str() {
            "--ratings" => v.ratings = next().into(),
            "--buff" => v.buff = next().into(),
            "--config" => v.config = next().into(),
            "--out" => v.out = next().into(),
            "--rebuild" => v.rebuild = true,
            _ => panic!("unknown argument {x}"),
        }
    }
    v
}

fn modified(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// Whether the index at `out` must be built again.
fn stale(a: &Args) -> bool {
    let other = schema::version(&a.out).as_deref() != Some(schema::VERSION);
    // A fresh buff.db is a reason to rebuild too: it renames teams and moves rosters.
    let inputs = modified(&a.ratings).max(modified(&a.buff));
    a.rebuild || other || modified(&a.out) < inputs
}

fn main() {
    let a = args();
    if !stale(&a) {
        eprintln!("index: {} is current", a.out.display());
        return;
    }
    let d = data::read_display(&a.config);
    let (achieve, team_steps) = schema::site_reads(&data::open(&a.ratings));
    schema::ensure_ratings_indexes(&a.ratings);
    // The server keeps reading the old index while this one is written, so the
    // new file is built beside it and renamed into place only when complete.
    let mut tmp = a.out.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    index::build(&a.ratings, &a.buff, &tmp, &d, &index::Reads { achieve, team_steps });
    std::fs::rename(&tmp, &a.out)
        .unwrap_or_else(|e| panic!("rename {} to {}: {e}", tmp.display(), a.out.display()));
    eprintln!("index: wrote {}", a.out.display());
}
