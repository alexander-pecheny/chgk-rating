use crate::question_model::Players;

/// Rolling window of rated games, used both for the gauge and for the listing rule.
pub struct Activity {
    days: Vec<i32>,
    who: Vec<u32>,
    info: Vec<f64>,
    cursor: usize,
    pub count: Vec<u32>,
    /// Fisher information earned in the window, for the uncertainty diagnostics.
    pub fisher: Vec<f64>,
}

impl Activity {
    pub fn new(n_players: usize) -> Activity {
        Activity {
            days: Vec::new(),
            who: Vec::new(),
            info: Vec::new(),
            cursor: 0,
            count: vec![0; n_players],
            fisher: vec![0.0; n_players],
        }
    }

    pub fn push(&mut self, day: i32, player: u32, info: f64) {
        self.days.push(day);
        self.who.push(player);
        self.info.push(info);
        self.count[player as usize] += 1;
        self.fisher[player as usize] += info;
    }

    pub fn expire_before(&mut self, day: i32) {
        while self.cursor < self.days.len() && self.days[self.cursor] < day {
            self.count[self.who[self.cursor] as usize] -= 1;
            self.fisher[self.who[self.cursor] as usize] -= self.info[self.cursor];
            self.cursor += 1;
        }
    }
}

/// Mean theta of the established players; DESIGN 3.5.
pub fn shift(players: &Players, activity: &Activity, n_est: u32) -> f64 {
    let mut sum = 0.0;
    let mut n: u32 = 0;
    for (i, &c) in activity.count.iter().enumerate() {
        if c >= n_est {
            sum += players.theta(i);
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}
