# Plan 7: the question model, fourth round — the metric's own shape, and geography

## Where round 3 left it

`out/q_ndcg/round3.md`: weighted test NDCG 0.9086 / @3 0.8111 against
rating.chgk.fun 0.9040 / 0.7997 (+0.0046 ± 0.0035) and chgk.quest
0.9011 / 0.7914 (+0.0075 ± 0.0040). Three rounds, 320 fits, seventeen
levers; the survivors are a per-player in-person effect, a pessimistic
quantile, a tempered roster estimate, and the editor memory. Every
per-player parameter, lineup chemistry, unrated training, series memory and
the collaboration constant are dead. The residual sits on strong, fully
measured teams in tight fields. Two directions are untried: making the
*ordering* target the metric rather than the mean strength, and facts about
the day (where it is, how tired the players are) rather than about the
players' history.

Same protocol as plans 3, 5 and 6: tune window decides, test reported once
per row, weights from `data/tournament_weights.csv`, paired SE vs round 3,
fun and quest; every change a switch with the shipped default; the frozen
mirror a frozen copy of `buff.db`;
refit round 3 from `config.toml` first and reproduce 0.9084 / 0.8108 on it.

## Levers

1. **Order by expected relevance, not by mean strength.** NDCG with
   relevance `1/place` pays for getting the top right. Each lineup has a
   mean `Q(L)` and a spread (the roster estimate's sd from the players' σ,
   `roster_sd` exists). Draw `n = 200` samples of every lineup's strength,
   rank each draw, and order the field by `E[1/place]` (also try `P(place =
   1)` and `E[1/place]` with the pairwise noise `D` added). This replaces the
   quantile `σ_c` with the same information used the way the metric wants
   it; try it with `σ_c` at 0, 0.75 and 1.5. Forecast-only; the published
   `forecast_place` column is what changes. It is the one lever here with a
   mechanism directly aimed at the loss: in a tight field the team with the
   wider spread has the better chance of first place.

2. **Home and away.** A team playing in its own town, or a lineup most of
   whose players live where the tournament is, against one that travelled.
   `tournaments.town_id`, `teams.town`, `tournament_results.team_current_town`
   and `players` (check for a town field) are in buff.db. Residual check
   first: mean residual of home teams vs visitors, overall and in the top
   fifth of fields. Then a forecast bonus `home_bonus` on the display scale
   (try 50, 100, 200) for lineups with ≥ half their players from the
   tournament's town, and a separate one for the team's town.

3. **Load.** Number of in-person tournaments a player played in the trailing
   7 and 30 days (a championship weekend often has three), and the day of a
   multi-day event. Residual check; then a per-player penalty proportional
   to the count, forecast-only.

4. **Season timing.** Residual by month of the season (August rust, May
   fatigue). If the check shows a pattern above noise, a per-month offset on
   `φ`'s use in the forecast. Cheap, probably nothing.

5. **Editor memory, tuned properly.** It is the one live lever of round 3
   and was shipped at its first interior optimum. A finer grid over
   `editor_memory` {750, 1000, 1250, 1500, 2000} × `ed_gate` {0.1, 0.2,
   0.3} × `ed_min_n` {1, 2, 3}; a half-life (decay of old entries, 2 and 4
   years); a team×editor version beside the player×editor one; and the
   memory keyed by the editors' *pair* when two editors co-write. Also check
   whether the memory should feed the in-person `φ` update (at a small
   rate) rather than only the forecast.

6. **Field-conditional quantile.** `σ_c` is one number. Let it depend on
   the field's tightness (the 1st-to-4th forecast gap in display points):
   `σ_c` for tight fields vs loose ones, two constants. Round 2's
   `tau_spread` lost, but that changed the roster temperature, not the
   quantile; the mechanism differs.

Run 1, 5 and 2 first. Budget ~100 fits. Ship only what beats round 3 on tune
by more than one SE without losing on test.

## Deliverables

`out/q_ndcg/round4.md` (`git add -f`) with the residual checks, the lever
table, the regenerated eight-row compare table with paired SEs, the
acceptance row (tests 1–7) beside round 3, top 20 teams and players beside
round 3; code and docs as before; commits on your branch ending with the
attribution lines in your instructions; do not merge.
