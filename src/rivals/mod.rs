//! The reimplementations of other production ratings: chgk.gg (`gg`),
//! rating.chgk.fun (`fun`), chgk.quest (`quest`) and the London Elo (`elo`). Each engine owns its own
//! rules and its own scope of tournaments; everything else is here. An engine
//! opens its state from the command line, rates one tournament at a time and
//! publishes a weekly release, and `drive` owns the calendar and the writer, so
//! the engines share one loop and one output path (DESIGN 6 schema).
use crate::output::Writer;
use std::error::Error;
use std::time::Instant;

pub mod elo;
pub mod fun;
pub mod gg;
mod gg_load;
pub mod quest;
mod shared;

pub use shared::*;

/// Days between two releases.
pub const WEEK: i32 = 7;

pub type Fallible<T> = Result<T, Box<dyn Error>>;

/// One release as the driver hands it to an engine: its sequence number from 1,
/// its day, and whether it is the last one the run will publish.
pub struct Release {
    pub id: i64,
    pub day: i32,
    pub last: bool,
}

/// The weekly calendar of an engine: the first release, and the day the tail of
/// releases runs to. `None` publishes the release in progress and stops.
pub struct Schedule {
    pub first_day: i32,
    pub last_day: Option<i32>,
    /// Commit the writer's transaction every this many releases.
    pub checkpoint_every: i64,
}

/// One rival system: its tournaments in the order it rates them, and the two
/// steps the driver interleaves.
pub trait Rival {
    fn schedule(&self) -> Schedule;
    fn out_path(&self) -> &str;
    fn n_tournaments(&self) -> usize;
    /// Whether tournament `i` belongs to a release later than the one on `day`.
    fn after(&self, i: usize, day: i32) -> bool;
    /// Rate tournament `i`; `next` is the release it will be published in.
    fn rate(&mut self, w: &Writer, i: usize, next: &Release) -> Fallible<()>;
    fn publish(&mut self, w: &Writer, r: &Release) -> Fallible<()>;
    /// Whatever is written once the calendar has run out.
    fn finish(&mut self, _w: &Writer, _releases: i64) -> Fallible<()> {
        Ok(())
    }
}

/// The systems a binary can run.
#[derive(Clone, Copy)]
pub enum System {
    Elo,
    Fun,
    Gg,
    Quest,
}

/// The shared `main` of `ext_elo`, `ext_fun`, `ext_gg` and `ext_quest`.
pub fn run(system: System) -> Fallible<()> {
    let args = Args::from_env();
    let started = Instant::now();
    let mut engine: Box<dyn Rival> = match system {
        System::Elo => Box::new(elo::Elo::open(&args)?),
        System::Fun => Box::new(fun::Fun::open(&args)?),
        System::Gg => Box::new(gg::Gg::open(&args)?),
        System::Quest => Box::new(quest::Quest::open(&args)?),
    };
    let w = Writer::create(engine.out_path())?;
    let releases = drive(engine.as_mut(), &w)?;
    engine.finish(&w, releases)?;
    w.finish()?;
    eprintln!("{releases} releases, done in {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}

/// Rate every tournament in order, publishing each release as soon as the next
/// tournament falls after it, then the tail of the calendar. Returns how many
/// releases were published.
pub fn drive(e: &mut dyn Rival, w: &Writer) -> Fallible<i64> {
    let s = e.schedule();
    let mut r = Release { id: 1, day: s.first_day, last: false };
    for i in 0..e.n_tournaments() {
        while e.after(i, r.day) {
            publish(e, w, &mut r, &s)?;
        }
        e.rate(w, i, &r)?;
    }
    let last = s.last_day.unwrap_or(r.day);
    while r.day <= last {
        publish(e, w, &mut r, &s)?;
    }
    Ok(r.id - 1)
}

fn publish(e: &mut dyn Rival, w: &Writer, r: &mut Release, s: &Schedule) -> Fallible<()> {
    r.last = s.last_day.is_some_and(|l| r.day + WEEK > l);
    e.publish(w, r)?;
    if r.id % s.checkpoint_every == 0 {
        w.checkpoint()?;
    }
    r.id += 1;
    r.day += WEEK;
    Ok(())
}
