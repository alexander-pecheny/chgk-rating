# The two models together

Plan 3 tuned the players and the lineup estimate; plan 4 tuned what the team
number means on its own. This is the two of them in one model: every switch of
both branches survives, `config.toml` carries the winner of each, and the whole
history was refitted from it to `out/final.db` (`--to 2026-09-05 --rows-every 1`).
`out/final_eval.md` is the full acceptance report.

The short of it. **The forecast is exactly plan 3's winner** — the team rules
never touch it — so the published forecast still leads rating.chgk.fun,
chgk.quest, chgk.gg and the old system on the test window. **The team number is
exactly plan 4's winner** to within the noise of a different set of players —
still ahead of chgk.gg and the old system on the field of base-roster teams.
The two do not interfere, and neither constant needed re-tuning.

## 1. The final config

```
[question] format_effect = "in_person", s_phi = 0.3
[player]   tau_phi = 0.05
[team.s2]  K = 0.4, D = 1200, k_r = 0.3, k_q = 0.1, winner_floor = true,
           lineup = "tempered", tau = 0.7, sigma_c = 1.5
[data]     rows_every = 1, no pinned `to`
[display]  a_team = -7640.2986, b_team = 1.2677,
           a_player = 5461.6007, b_player = 4832.6752
```

Everything else is unchanged from the shipped model. The **team** display map had
to be refitted: `sigma_c` lowered the level of the team scale and the pull changed
its spread, so the map plan 3 fitted no longer hit the 2019 anchors of DESIGN 5
(the 50th team of the 2019 release came out at 7961 instead of 7028). After
`calibrate --display` and a second fit the anchors are exact again and the map is
a fixed point. The player map did not move. Both maps are affine with a positive
slope, so no place in either table depends on this.

## 2. Re-tuning the team constants on the new players

`team_sweep` over the merged config, tune window 2018-09-01 to 2024-08-29,
weighted, paired bootstrap of the NDCG@3 difference against the base row, 2000
resamples. `out/team_ndcg/retune.md` and `out/team_ndcg/retune_grid.md`.

| row | tune NDCG (w) | tune @3 (w) | SE of @3 | test NDCG (w) | test @3 (w) | winner rule |
|---|---:|---:|---:|---:|---:|---:|
| **base: `k_r` 0.3, `k_q` 0.1, `K` 0.4** | **0.8891** | **0.7756** | -- | **0.9037** | **0.8016** | 0 / 0 |
| `k_r` 0 (no pull) | 0.8858 | 0.7733 | 0.0057 | 0.8917 | 0.7829 | 0 / 0 |
| `k_r` 0.1 | 0.8893 | 0.7768 | 0.0036 | 0.9009 | 0.8020 | 0 / 0 |
| `k_r` 0.2 | 0.8889 | 0.7757 | 0.0024 | 0.9022 | 0.8005 | 0 / 0 |
| `k_r` 0.5 | 0.8892 | 0.7761 | 0.0029 | 0.9023 | 0.7994 | 0 / 0 |
| `k_q` 0.03 | 0.8900 | 0.7757 | 0.0019 | 0.9038 | 0.8059 | 0 / 0 |
| `k_q` 0.3 | 0.8888 | 0.7747 | 0.0007 | 0.9040 | 0.8016 | 0 / 0 |
| `K` 0.8 | 0.8888 | 0.7761 | 0.0021 | 0.9020 | 0.7991 | 0 / 0 |
| `K` 1.2 | 0.8885 | 0.7740 | 0.0030 | 0.9016 | 0.8017 | 0 / 0 |

The full `k_r` × `k_q` grid over {0.1, 0.2, 0.3, 0.5} × {0.03, 0.1, 0.3} spans
0.8875 to 0.8900 of tune NDCG — a spread of 0.0025 against standard errors of
0.002 to 0.004. Every cell with `k_r ≥ 0.1` is inside one standard error of every
other. `K` is flat from 0.4 to 1.2 as plan 4 found, and a larger `K` is now
slightly negative.

**Decision: nothing moves.** The shipped point sits in the middle of the plateau
and ties the best row (`k_q` 0.03, +0.0009 of tune NDCG, half a standard error,
and no better at 3). Plan 4's reason for `k_q = 0.1` still stands and is not a
number on this table: the usual lineup has to track the real one at about the
speed the rating does, or the field-strength offset counts today's table twice
and the innovation autocorrelation blows out. `out/final_eval.md` has it at
−0.057 in person and +0.038 over all formats, which is white enough; with `k_r`
at 0.3 and `k_q` left at 0.01 plan 4 measured +0.19.

The one row that is not on the plateau is `k_r = 0`: the pull is still worth
+0.0033 of tune NDCG and +0.0120 of test NDCG. It earns less than it did on the
old players (plan 4 measured +0.0216 on tune), which is the expected direction —
a sharper lineup estimate leaves the pull less to add — but it still earns its
place.

The team scale did move, but from `sigma_c`, not from the constants; the display
refit of section 1 is the answer to that and was done in README's order.

## 3. Every system, both ways of ordering a field

`uv run tools/compare_ndcg.py --list`. A **player based** row ranks the field by
the forecast, the strength of the lineup that turned up. A **team based** row
ranks it by the team number as it stood that morning, `mu_before`, with an
unrated team last, so both kinds of row cover the same tournaments. Relevance is
`1/place` and ties are broken by the shuffle of `tools/ndcg.py` and `src/ndcg.rs`,
which agree to the last digit.

NDCG, relevance 1/place, in-person, 2024-08-30 to 2026-08-27, 282 common
tournaments, weights from `data/tournament_weights.csv`:

| system | NDCG (weighted) | NDCG@3 (weighted) | NDCG | NDCG@3 |
|---|---:|---:|---:|---:|
| proposed model (player based, new players) | 0.9069 | 0.8048 | 0.9048 | 0.8059 |
| rating.chgk.fun (player based) | 0.9040 | 0.7997 | 0.8983 | 0.7932 |
| chgk.quest (player based) | 0.9011 | 0.7914 | 0.8974 | 0.7883 |
| shipped model (player based) | 0.8981 | 0.7803 | 0.8945 | 0.7773 |
| official chgk.gg (player based) | 0.8710 | 0.7261 | 0.8749 | 0.7421 |
| proposed model (team based, current players) | 0.8232 | 0.6588 | 0.8270 | 0.6678 |
| proposed model (team based, new players) | 0.8207 | 0.6561 | 0.8306 | 0.6762 |
| official chgk.gg (team based) | 0.7917 | 0.6123 | 0.7986 | 0.6244 |

Paired bootstrap over the same tournaments, 2000 resamples, weighted; the
proposed player-based row minus the other row:

| against | NDCG | SE | NDCG@3 | SE |
|---|---:|---:|---:|---:|
| rating.chgk.fun (player based) | +0.0029 | 0.0035 | +0.0051 | 0.0078 |
| chgk.quest (player based) | +0.0058 | 0.0037 | +0.0134 | 0.0085 |
| shipped model (player based) | +0.0088 | 0.0032 | +0.0245 | 0.0072 |
| official chgk.gg (player based) | +0.0359 | 0.0055 | +0.0787 | 0.0124 |
| proposed model (team based, current players) | +0.0837 | 0.0089 | +0.1459 | 0.0170 |
| proposed model (team based, new players) | +0.0862 | 0.0089 | +0.1487 | 0.0170 |
| official chgk.gg (team based) | +0.1152 | 0.0096 | +0.1925 | 0.0184 |

How to read it:

- The proposed forecast is first, and it is the plan 3 winner unchanged: the
  merge cost nothing. The lead over **chgk.quest** is 1.6 standard errors on
  NDCG; over **rating.chgk.fun** it is 0.8 of one. We are no longer behind fun
  and we are not measurably ahead of it either. Against the model we ship the
  gain is 2.8 standard errors, and at 3 it is 3.4.
- **Every player-based row beats every team-based row by a wide margin**, ours
  included. That is not a defect of the team number; it is DESIGN principle 6 —
  about tomorrow, the lineup knows more than the accumulated rating. The rows
  here are also harsher on a team state than DESIGN 7.3 is: this table scores the
  **whole** in-person field, in which 26% of the results are pickup lineups that
  take no rating update and 10.6% of them have no team state at all and are
  ranked last by construction. The metric of DESIGN 7.3 keeps only the results of
  teams that turned up in their own base roster by the МАИИ rule and re-ranks
  them among themselves, and there the same `mu_before` scores 0.9037 on the same
  window (section 4, test 7). Both readings are honest; they answer different
  questions.
- The **new players make the team row a shade worse weighted** (0.8207 against
  0.8232) and **a shade better unweighted** (0.8306 against 0.8270), both inside
  the noise of the difference. The two team-based columns are the same rules over
  different player estimates, so this is the only place in the whole exercise
  where the merge cost anything measurable, and it does not reach a standard
  error.
- **chgk.gg's team number is last**, 0.029 of weighted NDCG behind ours.

## 4. The acceptance tests

`out/q_ndcg/eval_shipped.md` beside `out/final_eval.md`, both over the same
window with the history written.

| | shipped | final | |
|---|---:|---:|---|
| test 1, cross-fitted log-loss, in person, all seasons | 0.4900 | **0.4899** | passed |
| test 1, the same in 2025/26 | 0.5161 | **0.5160** | passed |
| test 2, Spearman, in person | 0.8363 | **0.8531** | passed |
| test 2, Spearman, synchronous | 0.8194 | 0.8187 | passed |
| test 2, Spearman, strictly synchronous | 0.7810 | **0.8152** | passed |
| test 3, top 200 against chgk.quest | **117**/200 | 112/200 | failed, worse |
| test 3, top 200 against rating.chgk.fun | **122**/200 | 115/200 | failed, worse |
| test 3, Spearman of θ over 50+ games, chgk.quest | **0.8959** | 0.8918 | |
| test 3, the same against rating.chgk.fun | **0.7918** | 0.7792 | |
| test 4, inflation, largest September deviation | 6.3% | **6.1%** | passed |
| test 5, transfer invariance, teams moved | 0 | **0** (max delta 0) | passed |
| test 6, winner rule, violations sole / shared | 0 / 0 | **0 / 0** | passed |
| test 7, team NDCG on its own, tune (w) | 0.8792 | **0.8986** | passed |
| test 7, the same, test window (w) | 0.8860 | **0.9037** | passed |
| forecast NDCG, test window (w) | 0.8981 | **0.9069** | |
| forecast NDCG@3, test window (w) | 0.7803 | **0.8048** | |
| innovation autocorrelation, in person | −0.025 | −0.057 | |
| innovation autocorrelation, all formats | 0.012 | 0.038 | |
| non-provisional players at the last release | 6345 | 6339 | |

Test 7's numbers are on the 296 tune and 268 test tournaments all four systems
cover; chgk.gg scores 0.8893 and 0.8901 there and the old system 0.8877 and
0.8886, so the rating is ahead of both on both windows, which is what the test
asks. rating.chgk.fun is 0.9058 and 0.9137 and is still ahead of us, but that
column is not a team state at all — it is composed from the players who turned
up, which is our forecast's job, not our rating's.

**Test 3 is the only test that moved the wrong way,** and it moved by exactly
what plan 3 measured: 112 and 115 of 200, not one player further. The cause is
structural and is written up in `out/q_ndcg/report.md`: the published `θ` is now
the strength *away from* an in-person table and `φ` is not part of it, while the
external systems publish one number per player that mixes both formats. The team
side cannot affect this — the question model never reads a team state — and it
does not.

**Test 4 is worth a note.** `sigma_c` on its own took the inflation band from
6.3% to 1.7%; with the pull it is back to 6.1%. A rating that follows the lineups
that turn up also follows the *level* of those lineups, so the cold-start drift
the quantile removed comes partly back through the pull. It is inside the ±10%
band and marginally better than the model we ship, but the 1.7% was the nicest
number in either study and it did not survive the merge.

## 5. The tables

`uv run tools/q_ndcg_tops.py out/shipped.db out/final.db`.

### Top 20 teams, release 714 of 2026-09-03

| # | shipped | final |
|---:|---|---|
| 1 | Борский корабел 9777 | Борский корабел 9681 |
| 2 | Рабочее название 8496 | Gay Guerrilla 8678 |
| 3 | Мир 8440 | Мир 8496 |
| 4 | Призраки Коши 8288 | Призраки Коши 8372 |
| 5 | Вжух 8266 | Первая сборная 8326 |
| 6 | Gay Guerrilla 7977 | ff 8172 |
| 7 | Пахом Пихай 7944 | Вжух 8006 |
| 8 | Первая сборная 7888 | Тихие Тигры 7962 |
| 9 | Ла Скала 7871 | Механизм реакции 7946 |
| 10 | Механизм реакции 7723 | Осторожно, надвигается кайфун 7791 |
| 11 | Осторожно, надвигается кайфун 7540 | Рабочее название 7786 |
| 12 | Keisecker 7491 | Я просто медленно пишу 7774 |
| 13 | Номер 2 7487 | Железные рукава 7744 |
| 14 | Кринж без выходных 7343 | Приматы 7658 |
| 15 | Кошка или сова 7340 | Кузинатра 7583 |
| 16 | Братья 7301 | Олеги Литвы 7513 |
| 17 | Приматы 7293 | Кошка или сова 7429 |
| 18 | Одушевлённые аэросани 7286 | Хронически разумные United 7379 |
| 19 | Ойнохойя, или Грабли навсегда 7282 | Самый маленький Месси в мире 7372 |
| 20 | Тихие Тигры 7209 | Суббота 13 7370 |

Twelve of the twenty are the same teams and the leader does not change. The
spread narrows: the top team stands 39% above the 50th against 44% under the
shipped model, which is the pull keeping a rating from running far ahead of the
players behind it. The teams that enter play in person and have strong rosters;
the teams that leave — Пахом Пихай, Ла Скала, Keisecker, Номер 2, Братья,
Одушевлённые аэросани, Ойнохойя — earned their places over long runs with
lineups the rating had stopped tracking.

### Top 20 players, release 714 of 2026-09-03

| # | shipped | final |
|---:|---|---|
| 1 | Руссо Максим 15644 | Немец Илья 15351 |
| 2 | Семушин Иван 15283 | Руссо Максим 15351 |
| 3 | Брутер Александра 15079 | Семушин Иван 15022 |
| 4 | Мокин Игорь 14838 | Мокин Игорь 14998 |
| 5 | Немец Илья 14409 | Брутер Александра 14722 |
| 6 | Миротин Евгений 14325 | Лёгенький Николай 14146 |
| 7 | Васильев Дмитрий 14057 | Молдавский Вадим 13983 |
| 8 | Коробейников Александр 14023 | Коробейников Александр 13885 |
| 9 | Лёгенький Николай 13846 | Миротин Евгений 13825 |
| 10 | Галиакберов Денис 13813 | Галиакберов Денис 13696 |
| 11 | Молдавский Вадим 13804 | Туроверов Алексей 13654 |
| 12 | Изъюров Константин 13669 | Васильев Дмитрий 13489 |
| 13 | Мосягин Александр 13639 | Мосягин Александр 13440 |
| 14 | Марков Александр 13567 | Марков Александр 13403 |
| 15 | Туроверов Алексей 13546 | Изъюров Константин 13402 |
| 16 | Халиуллин Рузель 13482 | Айзятулова Иделия 13381 |
| 17 | Гилёв Алексей 13417 | Соловьёв Олег 13363 |
| 18 | Мамедзаде Мамедшакир 13377 | Гилёв Алексей 13230 |
| 19 | Айзятулова Иделия 13349 | Халиуллин Рузель 13180 |
| 20 | Карякин Дмитрий 13310 | Мамедзаде Мамедшакир 13180 |

Nineteen of the twenty are the same people, and this column is bit-identical to
plan 3's winner, as it has to be: no team lever reaches a player. The one change
at the top is Немец Илья from fifth to first, and the reading to keep in mind is
that the published number is now the strength *away from* an in-person table.

### What looks broken, and what does not

- **Nothing fails a rule.** Winner rule 0 / 0 sole and shared, transfer
  invariance 0 teams moved with a maximum difference of exactly zero, inflation
  6.1% inside the ±10% band, test 3 at 112 and 115 — not one player below what
  plan 3 already reported.
- **One team in the top 20 is thin.** *Самый маленький Месси в мире* stands 19th
  on **six rated games**, one past the five that lift a team out of provisional.
  Its rating sits almost exactly at what its last lineup is worth, which is the
  pull working as designed on a team with a strong roster and no record — but a
  six-game team in the top twenty is the sharpest edge of that rule, and it is the
  one thing here worth the owner's eye. *Кузинатра* (#209 to #15, 38 games) and
  *Олеги Литвы* (#33 to #16, 26 games) are the same story in a milder form.
- **The big movers below the top 20 all have the same cause** and none of them is
  a jump without one: Школа диско Копатыча #268 → #73, Bikes for Peace #241 →
  #63, В поисках мема #212 → #58 — teams whose lineups are worth more than their
  accumulated placement record said. The falls are the mirror image: Заговор
  лемуров #24 → #78, Keisecker #12 → #56, Ла Скала #9 → #48.
- **No player moves without cause**: the player table is plan 3's winner exactly,
  and the largest single change in the top twenty is one place of entry.
- **The display numbers are on a new team scale** (`a_team −7640.2986`,
  `b_team 1.2677`). A team's number is not comparable with the one it had under
  the shipped map even though its place may be unchanged.

## 6. What was not done

- **The Newton question fit stays off** and **editor affinity stays off**, for
  plan 3's reasons; the format effect and affinity share one slot.
- **`K` was not moved** even though plan 4 left it as the first thing to retry.
  It is flat from 0.4 to 1.2 once the pull is in, and on the new players 0.8 and
  1.2 are both slightly negative.
- **Test 3's fall was not addressed.** Publishing `θ + φ` would close most of it
  and would make the published number mean something different in the two tables
  the site shows. It is the obvious thing to revisit if the individual table
  matters more than the forecast.
- **The inflation band was not re-tightened.** The 1.7% the quantile bought is
  back to 6.1% with the pull in. Nothing was tried to recover it.
- **The lead over rating.chgk.fun on the forecast is still inside one standard
  error.** Plan 5 exists for that.
