//! The achievement rating, the site's team rating: a team's number is what its
//! best recent results were worth, and nothing else.
//!
//! ```text
//! perf    = how the team played: from the questions taken on the package
//!           (score), from the place against the field's lineups (place), or a
//!           blend of the two
//! value   = w_format · w_field · max(0, perf)
//! w_field = clamp(((top-k lineup mean − field_zero) / (field_ref − field_zero))^gamma, field_min, 1)
//! rating  = Σ best top_x of value · fade(age) / slots
//! ```
//!
//! Two stages. [`freeze`] runs the player pass once and writes each result's
//! values, which never change after the day it was played, into
//! `results.tsv`; [`save_lists`] writes the names and rosters beside it. Then
//! [`Lab::load`] reads the three files, [`Lab::book`] values every result
//! under a set of [`Knobs`], and the [`Book`] gives the team metric and the
//! table; [`publish`] writes the table into a fit's database and [`search`]
//! looks for better knobs.

mod book;
mod freeze;
mod frozen;
mod knobs;
mod lab;
mod publish;
mod search;
mod value;

pub use book::{Book, Standing};
pub use freeze::{freeze, player_strengths, save_lists, Freezer};
pub use knobs::Knobs;
pub use lab::Lab;
pub use publish::publish;
pub use search::{search, SearchOpts};
