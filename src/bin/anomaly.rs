//! Does the rating catch cheats?
//!
//! Cheating is close to impossible at an in-person tournament, awkward at a
//! synchronous one and trivial when the synchronous circuit is played at home,
//! as it was through 2020 and 2021. So a cheat's strength depends on the format
//! and an honest player's does not, and the rating stores enough to measure it.
//!
//! Two statistics per result, both centred on the mean of results of the same
//! kind, so the systematic differences between formats divide out:
//!
//! * **surprise** — `ln(forecast place) − ln(actual place)`. Available for every
//!   result, including the ones flagged `inRating = 0`, where the forecast is
//!   made from the lineup's individual ratings alone. That is exactly the
//!   question a cheat fails: given these people, where should this team finish?
//! * **residual** — `s* − s_r`, how far above the sum of its parts the team
//!   actually played, on the θ scale. `s*` is recovered by inverting the stored
//!   `z`. Only rated results carry it, but it is in the model's own units.
//!
//! `--profile ID` then reads the masks and asks the independent question: when a
//! suspect's team beat the field, did it beat them on the questions everyone
//! else took, or on the hard ones?

use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;

const SIGMA_MODEL: f64 = 0.3669;
const COVID: (&str, &str) = ("2020-03-01", "2021-12-31");

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    InPerson,
    Sync,
    StrictSync,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        match s {
            "in-person" => Some(Kind::InPerson),
            "sync" => Some(Kind::Sync),
            "strict-sync" => Some(Kind::StrictSync),
            _ => None,
        }
    }
    fn online(self) -> bool {
        self != Kind::InPerson
    }
}

struct Event {
    kind: Kind,
    covid: bool,
    resid: Option<f64>,
    surprise: Option<f64>,
}

#[derive(Default, Clone, Copy)]
struct Acc {
    n: f64,
    sum: f64,
    sum2: f64,
}

impl Acc {
    fn push(&mut self, x: f64) {
        self.n += 1.0;
        self.sum += x;
        self.sum2 += x * x;
    }
    fn mean(&self) -> f64 {
        if self.n > 0.0 {
            self.sum / self.n
        } else {
            f64::NAN
        }
    }
    fn sd(&self) -> f64 {
        if self.n < 2.0 {
            return f64::NAN;
        }
        (self.sum2 / self.n - self.mean() * self.mean()).max(0.0).sqrt()
    }
    /// Standard error of the mean, with the spread floored at half the
    /// population's, so a player with two results cannot look certain.
    fn se(&self, pop_sd: f64) -> f64 {
        if self.n < 1.0 {
            return f64::INFINITY;
        }
        let sd = if self.n < 3.0 { pop_sd } else { self.sd().max(pop_sd * 0.5) };
        sd / self.n.sqrt()
    }
    fn t(&self, pop_sd: f64) -> f64 {
        self.mean() / self.se(pop_sd)
    }
}

#[derive(Default)]
struct Player {
    name: String,
    rank: Option<i64>,
    display: f64,
    games: i64,
    /// surprise, split by where it was earned
    s_ip: Acc,
    s_on: Acc,
    s_covid: Acc,
    /// residual, the same split
    r_ip: Acc,
    r_on: Acc,
}

fn open(path: &str) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("open {path}: {e}"))
}

fn main() {
    let mut a = std::env::args().skip(1);
    let mut ratings = "out/ratings.db".to_string();
    let mut web = "out/web.db".to_string();
    let mut buff = "buff.db".to_string();
    let mut top = 200usize;
    let mut masks = false;
    let mut who: Vec<i64> = Vec::new();
    let mut profile: Vec<i64> = Vec::new();
    while let Some(x) = a.next() {
        match x.as_str() {
            "--db" => ratings = a.next().unwrap(),
            "--web" => web = a.next().unwrap(),
            "--buff" => buff = a.next().unwrap(),
            "--top" => top = a.next().unwrap().parse().unwrap(),
            "--masks" => masks = true,
            "--who" => {
                masks = true;
                who.push(a.next().unwrap().parse().unwrap());
            }
            "--profile" => profile.push(a.next().unwrap().parse().unwrap()),
            _ => panic!("unknown argument {x}"),
        }
    }
    let db = open(&ratings);
    db.execute("ATTACH DATABASE ?1 AS x", [&web]).expect("attach the serving index");

    if masks {
        masks_mode(&db, &open(&buff), &who);
        return;
    }

    let (evs, pop) = load_events(&db);
    let sd_s = pop.0;
    let sd_r = pop.1;
    eprintln!("{} results, surprise sd {sd_s:.3}, residual sd {sd_r:.3}", evs.len());

    if !profile.is_empty() {
        let b = open(&buff);
        for id in profile {
            profile_player(&db, &b, &evs, id);
        }
        return;
    }

    let players = attribute(&db, &evs);
    eprintln!("{} players with attributable results", players.len());
    let top_n: Vec<&Player> = {
        let mut v: Vec<&Player> =
            players.values().filter(|p| p.rank.map_or(false, |r| r as usize <= top)).collect();
        v.sort_by_key(|p| p.rank.unwrap());
        v
    };

    println!("# Ищем читеров в собственных числах рейтинга\n");
    println!(
        "Списать на очном турнире почти невозможно, на синхроне трудно, дома за монитором — \
         тривиально. Поэтому у читера сила зависит от формата, а у честного игрока нет.\n"
    );

    // ── 1 ────────────────────────────────────────────────────────────────────
    println!("## 1. Топ-{top}: рейтинг, не подтверждённый за столом\n");
    println!(
        "Игроки текущего топа, у которых почти нет очных турниров, отсортированные по тому, \
         насколько их команды обыгрывают собственный прогноз на синхроне.\n"
    );
    let mut thin: Vec<&&Player> = top_n.iter().filter(|p| p.s_ip.n <= 4.0).collect();
    thin.sort_by(|a, b| b.s_on.t(sd_s).total_cmp(&a.s_on.t(sd_s)));
    surprise_table(&thin, sd_s);

    // How rare a mean that large is, among players with a comparable number of
    // online results. Without this the number above is unreadable.
    let mut peers: Vec<f64> = players
        .values()
        .filter(|p| (10.0..=30.0).contains(&p.s_on.n))
        .map(|p| p.s_on.mean())
        .collect();
    peers.sort_by(f64::total_cmp);
    let q = |f: f64| peers[((peers.len() - 1) as f64 * f) as usize];
    println!(
        "\nСреди {} игроков с 10–30 онлайн-результатами среднее превышение прогноза \
         распределено так: медиана {:+.3}, 95-й процентиль {:+.3}, 99-й {:+.3}, \
         99.9-й {:+.3}, максимум {:+.3}.",
        peers.len(),
        q(0.5),
        q(0.95),
        q(0.99),
        q(0.999),
        peers[peers.len() - 1]
    );

    // ── 2 ────────────────────────────────────────────────────────────────────
    println!("\n## 2. Топ-{top}: разрыв между синхроном и очными\n");
    println!(
        "У кого разница «обыгрываю прогноз онлайн, не обыгрываю очно» больше всего. \
         Нужно не меньше десяти турниров каждого вида.\n"
    );
    let mut gap: Vec<(&&Player, f64, f64)> = top_n
        .iter()
        .filter(|p| p.s_ip.n >= 10.0 && p.s_on.n >= 10.0)
        .map(|p| {
            let g = p.s_on.mean() - p.s_ip.mean();
            let se = (p.s_on.se(sd_s).powi(2) + p.s_ip.se(sd_s).powi(2)).sqrt();
            (p, g, g / se)
        })
        .collect();
    gap.sort_by(|a, b| b.2.total_cmp(&a.2));
    println!("| Игрок | Рейтинг | Место | Очно n | Очно | Онлайн n | Онлайн | Разрыв | t |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|--:|");
    for (p, g, t) in gap.iter().take(15) {
        println!(
            "| {} | {} | {} | {:.0} | {:+.3} | {:.0} | {:+.3} | {:+.3} | {:.1} |",
            p.name,
            fmt(p.display),
            p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            p.s_ip.n,
            p.s_ip.mean(),
            p.s_on.n,
            p.s_on.mean(),
            g,
            t
        );
    }

    // ── 3 ────────────────────────────────────────────────────────────────────
    println!("\n## 3. Ковидный онлайн: {} — {}\n", COVID.0, COVID.1);
    println!(
        "Синхроны этих двух лет играли из дома. Игроки, чьи команды тогда обыгрывали прогноз \
         сильнее всего, — и та же величина на их очных турнирах в другие годы.\n"
    );
    let mut covid: Vec<&Player> = players
        .values()
        .filter(|p| p.s_covid.n >= 10.0 && p.s_ip.n >= 10.0)
        .collect();
    covid.sort_by(|a, b| b.s_covid.t(sd_s).total_cmp(&a.s_covid.t(sd_s)));
    println!("| Игрок | Рейтинг | Место | Ковид n | Ковид | t | Очно n | Очно |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|");
    for p in covid.iter().take(20) {
        println!(
            "| {} | {} | {} | {:.0} | {:+.3} | {:.1} | {:.0} | {:+.3} |",
            p.name,
            fmt(p.display),
            p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            p.s_covid.n,
            p.s_covid.mean(),
            p.s_covid.t(sd_s),
            p.s_ip.n,
            p.s_ip.mean()
        );
    }

    // ── 4 ────────────────────────────────────────────────────────────────────
    println!("\n## 4. Ковидный онлайн: кто именно тогда вырос\n");
    println!(
        "То же, но с требованием, чтобы очный результат был отрицательным: играл дома сильно \
         выше прогноза, за столом — ниже.\n"
    );
    let mut both: Vec<&Player> = players
        .values()
        .filter(|p| p.s_covid.n >= 10.0 && p.s_ip.n >= 15.0 && p.s_ip.mean() < 0.0)
        .collect();
    both.sort_by(|a, b| {
        (b.s_covid.mean() - b.s_ip.mean()).total_cmp(&(a.s_covid.mean() - a.s_ip.mean()))
    });
    println!("| Игрок | Рейтинг | Место | Ковид n | Ковид | Очно n | Очно | Разрыв |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|");
    for p in both.iter().take(20) {
        println!(
            "| {} | {} | {} | {:.0} | {:+.3} | {:.0} | {:+.3} | {:+.3} |",
            p.name,
            fmt(p.display),
            p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            p.s_covid.n,
            p.s_covid.mean(),
            p.s_ip.n,
            p.s_ip.mean(),
            p.s_covid.mean() - p.s_ip.mean()
        );
    }

    // ── 5 ────────────────────────────────────────────────────────────────────
    println!("\n## 5. Для сверки: тот же разрыв в единицах модели\n");
    println!(
        "Остаток `s* − s_r` — насколько команда сыграла выше суммы своих частей на шкале θ. \
         Считается только по зачтённым результатам, поэтому выборка меньше.\n"
    );
    let mut rgap: Vec<(&&Player, f64, f64)> = top_n
        .iter()
        .filter(|p| p.r_ip.n >= 10.0 && p.r_on.n >= 10.0)
        .map(|p| {
            let g = p.r_on.mean() - p.r_ip.mean();
            let se = (p.r_on.se(sd_r).powi(2) + p.r_ip.se(sd_r).powi(2)).sqrt();
            (p, g, g / se)
        })
        .collect();
    rgap.sort_by(|a, b| b.2.total_cmp(&a.2));
    println!("| Игрок | Рейтинг | Место | Очно n | Очно | Онлайн n | Онлайн | Разрыв | t |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|--:|");
    for (p, g, t) in rgap.iter().take(15) {
        println!(
            "| {} | {} | {} | {:.0} | {:+.3} | {:.0} | {:+.3} | {:+.3} | {:.1} |",
            p.name,
            fmt(p.display),
            p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            p.r_ip.n,
            p.r_ip.mean(),
            p.r_on.n,
            p.r_on.mean(),
            g,
            t
        );
    }
}

fn fmt(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.0}")
    } else {
        "—".into()
    }
}

fn surprise_table(rows: &[&&Player], sd: f64) {
    println!("| Игрок | Рейтинг | Место | Игр | Очно n | Онлайн n | Онлайн | t |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|");
    for p in rows.iter().take(20) {
        println!(
            "| {} | {} | {} | {} | {:.0} | {:.0} | {:+.3} | {:.1} |",
            p.name,
            fmt(p.display),
            p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            p.games,
            p.s_ip.n,
            p.s_on.n,
            p.s_on.mean(),
            p.s_on.t(sd)
        );
    }
}

/// Every result, with both statistics centred on the mean of its own cell —
/// format crossed with whether the result counted for the team's rating.
fn load_events(db: &Connection) -> (HashMap<(i64, i64), Event>, (f64, f64)) {
    let mut raw: Vec<((i64, i64), Kind, bool, bool, Option<f64>, Option<f64>)> = Vec::new();
    let mut st = db
        .prepare(
            "select v.tournament_id, v.team_id, t.type, t.date, v.rated, \
             v.forecast_place, v.actual_place, v.x, v.z, v.sigma_before, v.s_r \
             from team_events v join x.tournament t on t.id = v.tournament_id",
        )
        .unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let Some(kind) = Kind::parse(&r.get::<_, String>(2).unwrap_or_default()) else { continue };
        let date: String = r.get(3).unwrap_or_default();
        let rated = r.get::<_, Option<i64>>(4).unwrap().unwrap_or(1) != 0;
        let num = |i: usize| r.get::<_, Option<f64>>(i).ok().flatten();
        let surprise = match (num(5), num(6)) {
            (Some(f), Some(a)) if f > 0.0 && a > 0.0 => Some(f.ln() - a.ln()),
            _ => None,
        };
        let resid = match (num(7), num(8), num(9), num(10)) {
            (Some(x), Some(z), Some(sb), Some(sr)) => {
                Some(x + z * (sb * sb + SIGMA_MODEL * SIGMA_MODEL).sqrt() - sr)
            }
            _ => None,
        };
        raw.push((
            (r.get(0).unwrap(), r.get(1).unwrap()),
            kind,
            date.as_str() >= COVID.0 && date.as_str() <= COVID.1,
            rated,
            surprise.filter(|v| v.is_finite()),
            resid.filter(|v| v.is_finite()),
        ));
    }

    let mut cs: HashMap<(Kind, bool), Acc> = HashMap::new();
    let mut cr: HashMap<Kind, Acc> = HashMap::new();
    for e in &raw {
        if let Some(v) = e.4 {
            cs.entry((e.1, e.3)).or_default().push(v);
        }
        if let Some(v) = e.5 {
            cr.entry(e.1).or_default().push(v);
        }
    }
    let all_s: f64 = {
        let mut a = Acc::default();
        for e in &raw {
            if let Some(v) = e.4 {
                a.push(v - cs[&(e.1, e.3)].mean());
            }
        }
        a.sd()
    };
    let all_r: f64 = {
        let mut a = Acc::default();
        for e in &raw {
            if let Some(v) = e.5 {
                a.push(v - cr[&e.1].mean());
            }
        }
        a.sd()
    };
    for (k, v) in &cs {
        eprintln!(
            "  {} {}: n {:.0}, mean surprise {:+.3}",
            match k.0 {
                Kind::InPerson => "in person",
                Kind::Sync => "sync     ",
                Kind::StrictSync => "strict   ",
            },
            if k.1 { "rated  " } else { "unrated" },
            v.n,
            v.mean()
        );
    }

    let evs = raw
        .into_iter()
        .map(|e| {
            (
                e.0,
                Event {
                    kind: e.1,
                    covid: e.2,
                    surprise: e.4.map(|v| v - cs[&(e.1, e.3)].mean()),
                    resid: e.5.map(|v| v - cr[&e.1].mean()),
                },
            )
        })
        .collect();
    (evs, (all_s, all_r))
}

fn attribute(db: &Connection, evs: &HashMap<(i64, i64), Event>) -> HashMap<i64, Player> {
    let mut players: HashMap<i64, Player> = HashMap::with_capacity(60_000);
    let mut st = db
        .prepare(
            "select g.player_id, g.tournament_id, g.team_id, p.name, p.games, k.rank, k.display \
             from x.player_game g join x.player p on p.id = g.player_id \
             left join x.player_rank k on k.player_id = g.player_id",
        )
        .unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let Some(ev) = evs.get(&(r.get(1).unwrap(), r.get(2).unwrap())) else { continue };
        let e = players.entry(r.get(0).unwrap()).or_insert_with(|| Player {
            name: r.get::<_, Option<String>>(3).unwrap().unwrap_or_default(),
            rank: r.get(5).unwrap(),
            display: r.get::<_, Option<f64>>(6).unwrap().unwrap_or(f64::NAN),
            games: r.get::<_, Option<i64>>(4).unwrap().unwrap_or(0),
            ..Default::default()
        });
        if let Some(v) = ev.surprise {
            if ev.kind.online() {
                e.s_on.push(v);
                if ev.covid {
                    e.s_covid.push(v);
                }
            } else {
                e.s_ip.push(v);
            }
        }
        if let Some(v) = ev.resid {
            if ev.kind.online() {
                e.r_on.push(v);
            } else {
                e.r_ip.push(v);
            }
        }
    }
    players
}

/// One player's results, and the independent mask test: on the questions the
/// field found hardest, how far above the field did this team score?
fn profile_player(db: &Connection, buff: &Connection, evs: &HashMap<(i64, i64), Event>, id: i64) {
    let name: String = db
        .query_row("select name from x.player where id = ?1", [id], |r| r.get(0))
        .unwrap_or_else(|_| format!("игрок {id}"));
    println!("\n## {name} (id {id})\n");
    println!(
        "| Дата | Турнир | Тип | Зач. | Команда | Прогноз | Место | Взято | Лёгкие | Трудные |"
    );
    println!("|---|---|---|---|---|--:|--:|--:|--:|--:|");

    let mut st = db
        .prepare(
            "select g.tournament_id, t.name, t.date, t.type, g.team_id, m.name, \
             v.forecast_place, v.actual_place, v.rated \
             from x.player_game g join x.tournament t on t.id = g.tournament_id \
             left join x.team m on m.id = g.team_id \
             left join team_events v on v.tournament_id = g.tournament_id and v.team_id = g.team_id \
             where g.player_id = ?1 order by t.date",
        )
        .unwrap();
    let games: Vec<(i64, String, String, String, i64, String, Option<f64>, Option<f64>, bool)> = st
        .query_map([id], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get(4)?,
                r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                r.get(6)?,
                r.get(7)?,
                r.get::<_, Option<i64>>(8)?.unwrap_or(1) != 0,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let mut easy = Acc::default();
    let mut hard = Acc::default();
    for g in &games {
        let edge = mask_edge(db, buff, g.0, g.4);
        if let Some((e, h)) = edge {
            easy.push(e);
            hard.push(h);
        }
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            g.2,
            g.1.chars().take(34).collect::<String>(),
            g.3,
            if g.8 { "да" } else { "нет" },
            g.5,
            g.6.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into()),
            g.7.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into()),
            evs.get(&(g.0, g.4))
                .and_then(|e| e.surprise)
                .map(|v| format!("{v:+.2}"))
                .unwrap_or_else(|| "—".into()),
            edge.map(|x| format!("{:+.2}", x.0)).unwrap_or_else(|| "—".into()),
            edge.map(|x| format!("{:+.2}", x.1)).unwrap_or_else(|| "—".into()),
        );
    }
    println!(
        "\nСредний отрыв от поля: на лёгкой трети пакета {:+.3}, на трудной {:+.3} ({:.0} турниров).",
        easy.mean(),
        hard.mean(),
        easy.n
    );
}

/// How far this team's take rate sits above the field's, on the easiest and the
/// hardest third of the package. A team that looks answers up gains where the
/// field cannot, so its edge is lopsided towards the hard third.
fn mask_edge(_db: &Connection, buff: &Connection, tid: i64, team: i64) -> Option<(f64, f64)> {
    let mut st = buff
        .prepare("select team_id, mask from tournament_results where id = ?1")
        .ok()?;
    let rows: Vec<(i64, String)> = st
        .query_map([tid], |r| {
            Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default()))
        })
        .ok()?
        .filter_map(|r| r.ok())
        .filter(|r| !r.1.is_empty())
        .collect();
    let mine = rows.iter().find(|r| r.0 == team)?.1.clone();
    let n_q = rows.iter().map(|r| r.1.len()).max()?;
    if n_q < 9 || rows.len() < 10 {
        return None;
    }

    // The field's own take rate is the difficulty ranking; no model needed.
    let mut take = vec![(0.0f64, 0.0f64); n_q];
    for (_, m) in &rows {
        for (q, c) in m.bytes().enumerate().take(n_q) {
            match c {
                b'1' => {
                    take[q].0 += 1.0;
                    take[q].1 += 1.0;
                }
                b'0' => take[q].1 += 1.0,
                _ => {}
            }
        }
    }
    let mut order: Vec<usize> = (0..n_q).filter(|&q| take[q].1 > 0.0).collect();
    order.sort_by(|&a, &b| (take[a].0 / take[a].1).total_cmp(&(take[b].0 / take[b].1)));
    let third = order.len() / 3;
    if third == 0 {
        return None;
    }
    let edge = |qs: &[usize]| -> f64 {
        let mut me = 0.0;
        let mut field = 0.0;
        let mut n = 0.0;
        for &q in qs {
            let Some(c) = mine.as_bytes().get(q) else { continue };
            if *c != b'1' && *c != b'0' {
                continue;
            }
            me += (*c == b'1') as i32 as f64;
            field += take[q].0 / take[q].1;
            n += 1.0;
        }
        if n > 0.0 {
            (me - field) / n
        } else {
            f64::NAN
        }
    };
    let hard = edge(&order[..third]);
    let easy = edge(&order[order.len() - third..]);
    (easy.is_finite() && hard.is_finite()).then_some((easy, hard))
}

// ── the mask test ────────────────────────────────────────────────────────────

/// A team's answers against the field's, on the third of the package the field
/// found hardest and the third it found easiest. This uses no rating at all: the
/// field's own take rate is the difficulty scale.
///
/// A strong team beats the field everywhere, so the hard-third edge on its own
/// is just strength. What is calibrated away is the hard-third edge a team with
/// this easy-third edge normally has — the easy third being the part of the
/// package a cheat has no reason to touch, because everyone already knows it.
/// The remainder is the surplus: points taken where nobody in the hall knew the
/// answer, by a team that was ordinary where everybody did.
///
/// Conditioning on the easy third rather than on the overall edge matters. A
/// team whose mask is misaligned with the question order looks flat across
/// difficulty — far below the field on the easy third, level with it on the
/// hard — and against an overall-edge baseline that reads as the strongest
/// signal in the data. Against the easy third it does not.
struct MaskResult {
    tid: i64,
    team: i64,
    kind: Kind,
    covid: bool,
    easy: f64,
    hard: f64,
}

fn masks_mode(db: &Connection, buff: &Connection, who: &[i64]) {
    let t0 = std::time::Instant::now();
    let mut meta: HashMap<i64, (Kind, bool)> = HashMap::new();
    {
        let mut st = db.prepare("select id, type, date from x.tournament").unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let Some(k) = Kind::parse(&r.get::<_, String>(1).unwrap_or_default()) else { continue };
            let d: String = r.get(2).unwrap_or_default();
            meta.insert(r.get(0).unwrap(), (k, d.as_str() >= COVID.0 && d.as_str() <= COVID.1));
        }
    }

    let mut by_tour: HashMap<i64, Vec<(i64, Vec<u8>)>> = HashMap::with_capacity(meta.len());
    {
        let mut st = buff
            .prepare("select id, team_id, mask from tournament_results where mask is not null")
            .unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let tid: i64 = match r.get(0) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !meta.contains_key(&tid) {
                continue;
            }
            let m: String = r.get::<_, Option<String>>(2).unwrap().unwrap_or_default();
            if m.len() < 12 {
                continue;
            }
            by_tour.entry(tid).or_default().push((r.get(1).unwrap_or(0), m.into_bytes()));
        }
    }
    eprintln!("masks for {} tournaments [{:.1}s]", by_tour.len(), t0.elapsed().as_secs_f64());

    let mut res: Vec<MaskResult> = Vec::with_capacity(400_000);
    for (tid, rows) in &by_tour {
        if rows.len() < 15 {
            continue;
        }
        let n_q = rows.iter().map(|r| r.1.len()).max().unwrap();
        let mut take = vec![(0.0f64, 0.0f64); n_q];
        for (_, m) in rows {
            for (q, c) in m.iter().enumerate() {
                match c {
                    b'1' => {
                        take[q].0 += 1.0;
                        take[q].1 += 1.0;
                    }
                    b'0' => take[q].1 += 1.0,
                    _ => {}
                }
            }
        }
        let mut order: Vec<usize> = (0..n_q).filter(|&q| take[q].1 >= 10.0).collect();
        if order.len() < 12 {
            continue;
        }
        order.sort_by(|&a, &b| (take[a].0 / take[a].1).total_cmp(&(take[b].0 / take[b].1)));
        let third = order.len() / 3;
        let (kind, covid) = meta[tid];
        for (team, m) in rows {
            let edge = |qs: &[usize]| -> Option<f64> {
                let (mut me, mut field, mut n) = (0.0, 0.0, 0.0);
                for &q in qs {
                    match m.get(q) {
                        Some(b'1') => me += 1.0,
                        Some(b'0') => {}
                        _ => continue,
                    }
                    field += take[q].0 / take[q].1;
                    n += 1.0;
                }
                (n >= 6.0).then(|| (me - field) / n)
            };
            let (Some(hard), Some(easy)) =
                (edge(&order[..third]), edge(&order[order.len() - third..]))
            else {
                continue;
            };
            res.push(MaskResult { tid: *tid, team: *team, kind, covid, easy, hard });
        }
    }
    eprintln!("{} results with usable masks [{:.1}s]", res.len(), t0.elapsed().as_secs_f64());

    // The hard-third edge a team with this easy-third edge normally has.
    let bin = |t: f64| ((t * 50.0).round() as i64).clamp(-35, 15);
    let mut curve: HashMap<i64, Acc> = HashMap::new();
    for r in &res {
        curve.entry(bin(r.easy)).or_default().push(r.hard);
    }
    let expect = |t: f64| curve.get(&bin(t)).filter(|a| a.n >= 30.0).map(|a| a.mean());
    let mut spread = Acc::default();
    for r in &res {
        if let Some(e) = expect(r.easy) {
            spread.push(r.hard - e);
        }
    }
    let sd = spread.sd();
    eprintln!("surplus sd {sd:.4} over {:.0} results", spread.n);

    let mut per_team: HashMap<i64, Acc> = HashMap::new();
    let mut per_result: HashMap<(i64, i64), f64> = HashMap::with_capacity(res.len());
    let mut per_team_covid: HashMap<i64, Acc> = HashMap::new();
    let mut per_team_ip: HashMap<i64, Acc> = HashMap::new();
    for r in &res {
        let Some(e) = expect(r.easy) else { continue };
        let s = r.hard - e;
        per_result.insert((r.tid, r.team), s);
        per_team.entry(r.team).or_default().push(s);
        if r.covid && r.kind.online() {
            per_team_covid.entry(r.team).or_default().push(s);
        }
        if !r.kind.online() {
            per_team_ip.entry(r.team).or_default().push(s);
        }
    }

    // ── players ──────────────────────────────────────────────────────────────
    #[derive(Default)]
    struct P {
        name: String,
        rank: Option<i64>,
        display: f64,
        ip: Acc,
        on: Acc,
        covid: Acc,
    }
    let mut ps: HashMap<i64, P> = HashMap::with_capacity(60_000);
    {
        let kinds: HashMap<i64, (Kind, bool)> = meta;
        let mut st = db
            .prepare(
                "select g.player_id, g.tournament_id, g.team_id, p.name, k.rank, k.display \
                 from x.player_game g join x.player p on p.id = g.player_id \
                 left join x.player_rank k on k.player_id = g.player_id",
            )
            .unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let tid: i64 = r.get(1).unwrap();
            let Some(&s) = per_result.get(&(tid, r.get(2).unwrap())) else { continue };
            let Some(&(kind, covid)) = kinds.get(&tid) else { continue };
            let e = ps.entry(r.get(0).unwrap()).or_insert_with(|| P {
                name: r.get::<_, Option<String>>(3).unwrap().unwrap_or_default(),
                rank: r.get(4).unwrap(),
                display: r.get::<_, Option<f64>>(5).unwrap().unwrap_or(f64::NAN),
                ..Default::default()
            });
            if kind.online() {
                e.on.push(s);
                if covid {
                    e.covid.push(s);
                }
            } else {
                e.ip.push(s);
            }
        }
    }

    if !who.is_empty() {
        println!("| Игрок | Место | Онлайн n | Избыток | t | Очно n | Очно |");
        println!("|---|--:|--:|--:|--:|--:|--:|");
        for id in who {
            let Some(p) = ps.get(id) else { continue };
            println!(
                "| {} | {} | {:.0} | {:+.3} | {:.1} | {:.0} | {} |",
                p.name,
                p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
                p.on.n,
                p.on.mean(),
                p.on.t(sd),
                p.ip.n,
                if p.ip.n > 0.0 { format!("{:+.3}", p.ip.mean()) } else { "—".into() }
            );
        }
        return;
    }

    println!("# Тест по маскам: где именно команда обыгрывает поле\n");
    println!(
        "Вопросы каждого пакета делятся на трети по доле взятия в поле. Для каждой команды \
         считается отрыв от поля на трудной трети и на лёгкой, и первый сравнивается с тем, \
         какой он обычно бывает при таком же отрыве на лёгкой. Лёгкую треть списывать незачем — \
         её и так берут все, — поэтому остаток и есть избыток на трудных вопросах. Рейтинг \
         в этом тесте не используется вообще; среднеквадратичный разброс остатка {:.3}.\n",
        sd
    );

    println!("## 1. Текущий топ-200: избыток на трудных вопросах онлайн\n");
    let mut v: Vec<&P> = ps
        .values()
        .filter(|p| p.rank.map_or(false, |r| r <= 200) && p.on.n >= 10.0)
        .collect();
    v.sort_by(|a, b| b.on.t(sd).total_cmp(&a.on.t(sd)));
    println!("| Игрок | Рейтинг | Место | Онлайн n | Избыток | t | Очно n | Избыток очно |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|");
    for p in v.iter().take(15) {
        println!(
            "| {} | {} | {} | {:.0} | {:+.3} | {:.1} | {:.0} | {} |",
            p.name,
            fmt(p.display),
            p.rank.unwrap(),
            p.on.n,
            p.on.mean(),
            p.on.t(sd),
            p.ip.n,
            if p.ip.n > 0.0 { format!("{:+.3}", p.ip.mean()) } else { "—".into() }
        );
    }

    println!("\n## 2. Ковидный онлайн ({} — {}): игроки\n", COVID.0, COVID.1);
    println!(
        "С поправкой на собственную силу: у сильного игрока избыток положителен и очно, \
         поэтому здесь сортировка по разнице «дома минус за столом», и очных турниров \
         требуется не меньше двадцати.\n"
    );
    let split_t = |p: &P, x: &Acc| {
        (x.mean() - p.ip.mean()) / (x.se(sd).powi(2) + p.ip.se(sd).powi(2)).sqrt()
    };
    let mut v: Vec<&P> = ps.values().filter(|p| p.covid.n >= 12.0 && p.ip.n >= 20.0).collect();
    // Everyone's hard-third surplus was higher in those two years, the top three
    // players in the country included, so the population's own shift is the zero
    // this table is read against.
    let shift = {
        let mut a = Acc::default();
        for p in &v {
            a.push(p.covid.mean() - p.ip.mean());
        }
        a
    };
    println!(
        "Средний сдвиг по всем {:.0} игрокам таблицы — {:+.3}: в ковид избыток вырос у всех, \
         включая первую тройку рейтинга. Интересны только те, кто заметно выше этого.\n",
        shift.n, shift.mean()
    );
    v.sort_by(|a, b| split_t(b, &b.covid).total_cmp(&split_t(a, &a.covid)));
    println!("| Игрок | Рейтинг | Место | Ковид n | Ковид | Очно n | Очно | Разрыв | t |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|--:|");
    for p in v.iter().take(20) {
        println!(
            "| {} | {} | {} | {:.0} | {:+.3} | {:.0} | {:+.3} | {:+.3} | {:.1} |",
            p.name,
            fmt(p.display),
            p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            p.covid.n,
            p.covid.mean(),
            p.ip.n,
            p.ip.mean(),
            p.covid.mean() - p.ip.mean(),
            split_t(p, &p.covid)
        );
    }

    println!("\n## 2b. Самый большой избыток онлайн среди всех игроков\n");
    println!("Без ограничения на текущий топ; двенадцать и более онлайн-турниров.\n");
    let mut v: Vec<&P> = ps.values().filter(|p| p.on.n >= 12.0).collect();
    v.sort_by(|a, b| b.on.t(sd).total_cmp(&a.on.t(sd)));
    println!("| Игрок | Рейтинг | Место | Онлайн n | Избыток | t | Очно n | Очно |");
    println!("|---|--:|--:|--:|--:|--:|--:|--:|");
    for p in v.iter().take(20) {
        println!(
            "| {} | {} | {} | {:.0} | {:+.3} | {:.1} | {:.0} | {} |",
            p.name,
            fmt(p.display),
            p.rank.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
            p.on.n,
            p.on.mean(),
            p.on.t(sd),
            p.ip.n,
            if p.ip.n > 0.0 { format!("{:+.3}", p.ip.mean()) } else { "—".into() }
        );
    }

    let mut all: Vec<f64> = ps
        .values()
        .filter(|p| p.on.n >= 12.0)
        .map(|p| p.on.mean())
        .collect();
    all.sort_by(f64::total_cmp);
    let q = |f: f64| all[((all.len() - 1) as f64 * f) as usize];
    println!(
        "\nПо всем {} игрокам с двенадцатью и более онлайн-турнирами избыток распределён так: \
         медиана {:+.3}, 99-й процентиль {:+.3}, 99.9-й {:+.3}, максимум {:+.3}.",
        all.len(),
        q(0.5),
        q(0.99),
        q(0.999),
        all[all.len() - 1]
    );

    let names: HashMap<i64, (String, String)> = {
        let mut st = db.prepare("select id, name, town from x.team").unwrap();
        let v = st
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    (
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ),
                ))
            })
            .unwrap();
        v.map(|x| x.unwrap()).collect()
    };
    println!("\n## 4. Команды: наибольший избыток на трудных вопросах за всё время\n");
    println!("Не меньше тридцати результатов с масками.\n");
    let mut w: Vec<(&i64, &Acc)> = per_team.iter().filter(|(_, a)| a.n >= 30.0).collect();
    w.sort_by(|a, b| b.1.t(sd).total_cmp(&a.1.t(sd)));
    println!("| Команда | Город | n | Избыток | t | Очно n | Очно | Первый | Последний |");
    println!("|---|---|--:|--:|--:|--:|--:|---|---|");
    for (id, a) in w.iter().take(20) {
        let ip = per_team_ip.get(*id).copied().unwrap_or_default();
        let (n, t) = names.get(*id).cloned().unwrap_or_default();
        let span: (String, String) = db
            .query_row(
                "select min(t.date), max(t.date) from x.result_ r \
                 join x.tournament t on t.id = r.tournament_id where r.team_id = ?1",
                [*id],
                |r| Ok((r.get(0).unwrap_or_default(), r.get(1).unwrap_or_default())),
            )
            .unwrap_or_default();
        println!(
            "| {} | {} | {:.0} | {:+.3} | {:.1} | {:.0} | {} | {} | {} |",
            n,
            t,
            a.n,
            a.mean(),
            a.t(sd),
            ip.n,
            if ip.n > 0.0 { format!("{:+.3}", ip.mean()) } else { "—".into() },
            span.0,
            span.1
        );
    }

    println!("\n## 3. Ковидный онлайн: команды\n");
    let mut v: Vec<(&i64, &Acc)> =
        per_team_covid.iter().filter(|(_, a)| a.n >= 12.0).collect();
    v.retain(|(id, _)| per_team_ip.get(*id).map_or(false, |c| c.n >= 12.0));
    v.sort_by(|a, b| {
        let t = |id: &i64, x: &Acc| {
            let ip = per_team_ip[id];
            (x.mean() - ip.mean()) / (x.se(sd).powi(2) + ip.se(sd).powi(2)).sqrt()
        };
        t(b.0, b.1).total_cmp(&t(a.0, a.1))
    });
    println!("| Команда | Город | Ковид n | Ковид | Очно n | Очно | Разрыв | Всего n |");
    println!("|---|---|--:|--:|--:|--:|--:|--:|");
    for (id, a) in v.iter().take(20) {
        let all = per_team.get(id).copied().unwrap_or_default();
        let ip = per_team_ip.get(id).copied().unwrap_or_default();
        let (n, t) = names.get(id).cloned().unwrap_or_default();
        println!(
            "| {} | {} | {:.0} | {:+.3} | {:.0} | {} | {:+.3} | {:.0} |",
            n,
            t,
            a.n,
            a.mean(),
            ip.n,
            if ip.n > 0.0 { format!("{:+.3}", ip.mean()) } else { "—".into() },
            a.mean() - if ip.n > 0.0 { ip.mean() } else { 0.0 },
            all.n
        );
    }
}
