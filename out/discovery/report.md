# Plan 8: feature discovery with a boosted ranker

Rounds 1 to 3 tested seventeen hypotheses one at a time and kept four. This
round went wide instead: **128 columns** of everything the pipeline can know
about a lineup at 09:00 on the morning of its tournament, a gradient-boosted
ranker trained on them over the tune window, and the importances read off it.

The answer is that there is no headroom, and the shape of the answer is sharper
than the number. **Five of the ranker's top six features are the lineup's own
position in its field by `Q(L)`** — the gap to the fourth-placed forecast, the
z-score, the rank, the gap to first — which is the shipped forecast written as a
rank rather than as a number. Every genuinely new signal in 128 columns is worth
at most +0.002 of weighted NDCG in permutation, against a paired standard error
of 0.0018. Dropping any family of features except the field context costs
nothing at all; dropping the field context costs 0.0090.

And the ranker does not beat the model. It is **+0.0027 ± 0.0018 on the tune
window and −0.0047 ± 0.0026 on the test window**, which is the two-window
protocol catching a ranker doing what a ranker does. Trained forward from round
3's own forecast the story is the same sign for sign: +0.0019 ± 0.0011 on tune,
−0.0046 ± 0.0020 on test. CatBoost's YetiRank, run as a cross-check, agrees:
+0.0005 on its validation fold and 0.9077 against 0.9084 on test.

Nothing ships. Two new switches are added at zero, where the fit is
bit-identical, because the distillation had to be measurable; both lose.

## 0. The protocol

Everything is on the frozen mirror
`/mnt/HC_Volume_106816389/build/herdr-worktrees/r2out/buff_frozen.db`, symlinked
as `buff.db`. The round-3 winner refitted from `config.toml` at `e1a18e3` with
`--to 2026-09-05 --rows-every 0`:

```
ndcg tune 0.8963 @3 0.7834  unweighted 0.9014 @3 0.7988  (650 tournaments)
ndcg test 0.9084 @3 0.8108  unweighted 0.9070 @3 0.8122  (284 tournaments)
```

which is `out/q_ndcg/round3.md` section 3 in every digit. Tune is 2018-09-01 to
2024-08-29 and decides; test is 2024-08-30 to 2026-08-27 and is read once per
row; the weights are frozen at `fffb987`; the paired bootstrap is 2000
resamples.

Every tune-window number below is **out of fold**. The 650 tune fields are split
into five, and each fifth is predicted by a model trained on everything dated
before the test window except that fifth — which includes the **1043 in-person
fields of 2013 to 2018** that neither window scores, so the ranker trains on
50 489 results and is judged on 18 651 it has not seen. The test window is
scored once, by a model retrained on everything before it.

Two details had to be right before any of this meant anything, and both were
wrong at first.

- **The label gain is the metric's own relevance.** LightGBM's NDCG is
  `2^label − 1` by default; here label `l` is place `11 − l` and its gain is set
  to `1 / place`, so the ranker optimises what it is judged on.
- **The early-stopping eval breaks ties the way the metric does.** The rows of a
  field are stored in finishing order, so a stable sort over a young ensemble —
  which puts whole blocks of a field on one score — reads the answer off the
  table. It scored an untrained model at 0.935 where the metric said 0.866, and
  early stopping then chose that one tree and stopped. Every group is
  pre-shuffled by `ndcg.Rng` seeded from its tournament id, exactly as
  `tools/ndcg.py` does.

## 1. The feature table

`fit --features out/discovery/features.db` writes one row per result of every
in-person field: **58 487 rows over 1986 fields**, 2013-01-04 to 2026-08-30, 138
columns of which **128 are features**. Four more are joined on in Python from
`out/ext/*.db`, so the ranker sees 132 and 127 of them without the external
ones. The writer is `src/features.rs` and it costs the fit about a second.

Turning it on changes nothing the model produces: the same fit writes a
`team_events` identical to `out/round3.db` row for row and column for column,
and the same 0.8963 / 0.9084. The row is built after the field has been scored
and before any forecast memory has absorbed it, which is where `fit` already
advances the editor memory and the pair counter.

| family | columns | what it is |
|---|---:|---|
| `lin_` lineup strength | 21 | `Q(L)` under five rules — the mean, chgk.gg's best-6, the roster estimate, and the tempered estimate at 0.5 and 0.7 — each at four pessimistic quantiles `σ_c` of 0, 0.75, 1.5 and 3, plus the sd of the tempered estimate |
| `agg_` player aggregates | 54 | fifteen per-player quantities summarised over the lineup by mean, smallest, largest, spread, top-3 sum and bottom-2 mean |
| `shp_` lineup shape | 12 | size, newcomers, the thinly measured, the chemistry `T(L)`, how much of the team's last lineup turned up, `Q(L) − Q̄` |
| `tm_` team state | 13 | `R`, `Q̄`, their difference, rated games, days since the last one, the run of surprises, the best and worst place of the year, the rating rank |
| `ed_` editor | 8 | the editor memory of DESIGN 4.8 gated and ungated with its pair counts, the team's own record under today's editors, and how many editors there are |
| `fld_` field context | 12 | where this lineup stands in this field: rank, z-score, gap to first and to fourth, teams within 100 points, the field's mean, spread and top-10 |
| `cal_` calendar and place | 7 | month, month of season, day of week, day of month, day of season, tournaments on the same date, home or away |
| `ext_` external systems | 5 | the old system's `predictedPosition`, chgk.gg's forecast and rating, rating.chgk.fun's forecast, chgk.quest's forecast — **for diagnosis only** |

One line each:

| column | what it is |
|---|---|
| `lin_mean_sc000` | Q(L) by the mean of the members, each member read at theta - 0.0 sigma |
| `lin_mean_sc075` | Q(L) by the mean of the members, each member read at theta - 0.75 sigma |
| `lin_mean_sc150` | Q(L) by the mean of the members, each member read at theta - 1.5 sigma |
| `lin_mean_sc300` | Q(L) by the mean of the members, each member read at theta - 3.0 sigma |
| `lin_best6_sc000` | Q(L) by chgk.gg's weighted top six, each member read at theta - 0.0 sigma |
| `lin_best6_sc075` | Q(L) by chgk.gg's weighted top six, each member read at theta - 0.75 sigma |
| `lin_best6_sc150` | Q(L) by chgk.gg's weighted top six, each member read at theta - 1.5 sigma |
| `lin_best6_sc300` | Q(L) by chgk.gg's weighted top six, each member read at theta - 3.0 sigma |
| `lin_rost_sc000` | Q(L) by the roster estimate, each member read at theta - 0.0 sigma |
| `lin_rost_sc075` | Q(L) by the roster estimate, each member read at theta - 0.75 sigma |
| `lin_rost_sc150` | Q(L) by the roster estimate, each member read at theta - 1.5 sigma |
| `lin_rost_sc300` | Q(L) by the roster estimate, each member read at theta - 3.0 sigma |
| `lin_t05_sc000` | Q(L) by the roster estimate tempered at 0.5, each member read at theta - 0.0 sigma |
| `lin_t05_sc075` | Q(L) by the roster estimate tempered at 0.5, each member read at theta - 0.75 sigma |
| `lin_t05_sc150` | Q(L) by the roster estimate tempered at 0.5, each member read at theta - 1.5 sigma |
| `lin_t05_sc300` | Q(L) by the roster estimate tempered at 0.5, each member read at theta - 3.0 sigma |
| `lin_t07_sc000` | Q(L) by the roster estimate tempered at 0.7, the shipped rule, each member read at theta - 0.0 sigma |
| `lin_t07_sc075` | Q(L) by the roster estimate tempered at 0.7, the shipped rule, each member read at theta - 0.75 sigma |
| `lin_t07_sc150` | Q(L) by the roster estimate tempered at 0.7, the shipped rule, each member read at theta - 1.5 sigma |
| `lin_t07_sc300` | Q(L) by the roster estimate tempered at 0.7, the shipped rule, each member read at theta - 3.0 sigma |
| `lin_roster_sd` | the sd of the tempered roster estimate, propagated from the members' sigma |
| `agg_theta_mean` | the mean over the lineup of theta |
| `agg_theta_min` | the smallest over the lineup of theta |
| `agg_theta_max` | the largest over the lineup of theta |
| `agg_theta_sd` | the spread over the lineup of theta |
| `agg_theta_top3` | the sum of the three largest over the lineup of theta |
| `agg_theta_bot2` | the mean of the two smallest over the lineup of theta |
| `agg_thetaphi_mean` | the mean over the lineup of theta + phi |
| `agg_thetaphi_min` | the smallest over the lineup of theta + phi |
| `agg_thetaphi_max` | the largest over the lineup of theta + phi |
| `agg_thetaphi_sd` | the spread over the lineup of theta + phi |
| `agg_thetaphi_top3` | the sum of the three largest over the lineup of theta + phi |
| `agg_thetaphi_bot2` | the mean of the two smallest over the lineup of theta + phi |
| `agg_sigma_mean` | the mean over the lineup of sigma as of that morning |
| `agg_sigma_min` | the smallest over the lineup of sigma as of that morning |
| `agg_sigma_max` | the largest over the lineup of sigma as of that morning |
| `agg_sigma_sd` | the spread over the lineup of sigma as of that morning |
| `agg_sigma_top3` | the sum of the three largest over the lineup of sigma as of that morning |
| `agg_sigma_bot2` | the mean of the two smallest over the lineup of sigma as of that morning |
| `agg_games_ip_mean` | the mean over the lineup of in-person games behind the player |
| `agg_games_ip_min` | the smallest over the lineup of in-person games behind the player |
| `agg_games_ip_max` | the largest over the lineup of in-person games behind the player |
| `agg_games_ip_sd` | the spread over the lineup of in-person games behind the player |
| `agg_days_ip_mean` | the mean over the lineup of days since the last in-person game |
| `agg_days_ip_min` | the smallest over the lineup of days since the last in-person game |
| `agg_days_ip_max` | the largest over the lineup of days since the last in-person game |
| `agg_days_ip_sd` | the spread over the lineup of days since the last in-person game |
| `agg_phi_mean` | the mean over the lineup of the format effect phi |
| `agg_phi_min` | the smallest over the lineup of the format effect phi |
| `agg_phi_max` | the largest over the lineup of the format effect phi |
| `agg_games_mean` | the mean over the lineup of rated games behind the player |
| `agg_games_min` | the smallest over the lineup of rated games behind the player |
| `agg_games_max` | the largest over the lineup of rated games behind the player |
| `agg_games_ip365_mean` | the mean over the lineup of in-person games in the last 365 days |
| `agg_games_ip365_min` | the smallest over the lineup of in-person games in the last 365 days |
| `agg_games_ip365_max` | the largest over the lineup of in-person games in the last 365 days |
| `agg_days_last_mean` | the mean over the lineup of days since the last game of any format |
| `agg_days_last_min` | the smallest over the lineup of days since the last game of any format |
| `agg_days_last_max` | the largest over the lineup of days since the last game of any format |
| `agg_vol_mean` | the mean over the lineup of the fitted volatility v |
| `agg_vol_min` | the smallest over the lineup of the fitted volatility v |
| `agg_vol_max` | the largest over the lineup of the fitted volatility v |
| `agg_first_age_mean` | the mean over the lineup of days since the player's first record |
| `agg_first_age_min` | the smallest over the lineup of days since the player's first record |
| `agg_first_age_max` | the largest over the lineup of days since the player's first record |
| `agg_share_ip_mean` | the mean over the lineup of the share of the player's games played in person |
| `agg_share_ip_min` | the smallest over the lineup of the share of the player's games played in person |
| `agg_share_ip_max` | the largest over the lineup of the share of the player's games played in person |
| `agg_dtheta90_mean` | the mean over the lineup of the change in theta over 90 days |
| `agg_dtheta90_min` | the smallest over the lineup of the change in theta over 90 days |
| `agg_dtheta90_max` | the largest over the lineup of the change in theta over 90 days |
| `agg_dtheta365_mean` | the mean over the lineup of the change in theta over 365 days |
| `agg_dtheta365_min` | the smallest over the lineup of the change in theta over 365 days |
| `agg_dtheta365_max` | the largest over the lineup of the change in theta over 365 days |
| `agg_sign5_mean` | the mean over the lineup of the sign of the gradient over the last five games |
| `shp_size` | seats filled in the lineup |
| `shp_newcomers` | members whose sigma is above the provisional cut |
| `shp_few_ip` | members with fewer than five in-person games |
| `shp_sync_only` | share of the lineup that has never played in person |
| `shp_no_history` | members with no rated game at all |
| `shp_n_last_rated` | members who played the team's last rated game |
| `shp_share_last_rated` | the same as a share of the lineup |
| `shp_n_last_ip` | members who played the team's last in-person game |
| `shp_chem_mean` | T(L), the mean over the pairs of log(1 + games together) |
| `shp_chem_min` | the same over the least-travelled pair |
| `shp_chem_max` | the same over the most-travelled pair |
| `shp_q_minus_qbar` | Q(L) less the team's usual lineup Qbar |
| `tm_has_state` | whether the team has a rating at all |
| `tm_r` | the team rating R that morning |
| `tm_qbar` | the usual lineup Qbar |
| `tm_r_minus_qbar` | how far the rating stands above the lineups that earned it |
| `tm_games` | rated games behind the team |
| `tm_days_since` | days since the team's last rated game |
| `tm_surp5_mean` | the mean of the team's last five in-person surprises |
| `tm_surp5_sd` | their spread |
| `tm_surp365_mean` | the mean surprise over the last 365 days |
| `tm_surp365_n` | how many in-person results that mean is over |
| `tm_best_place365` | the best place/n the team took in the last year |
| `tm_worst_place365` | the worst |
| `tm_rank` | the team's rank by rating among the teams active in the last year |
| `ed_player_gated` | the editor memory of DESIGN 4.8, gated at the top fifth |
| `ed_player_ungated` | the same with every player counted |
| `ed_pairs_gated` | qualifying (player, editor) pairs behind it |
| `ed_pairs_ungated` | the same without the gate |
| `ed_team_resid` | the team's own mean past residual under today's editors |
| `ed_team_n` | how many of today's editors the team has a record under |
| `ed_team_tournaments` | past fields the team has played under them |
| `ed_n_editors` | editors the package lists |
| `fld_n_teams` | teams in the field |
| `fld_weight` | the field's frozen weight from the CSV |
| `fld_rank_q` | the lineup's rank in the field by Q(L) |
| `fld_rank_frac` | the same over the field size |
| `fld_gap_first` | Q(L) less the field's best Q(L), in display points |
| `fld_gap_fourth` | Q(L) less the field's fourth |
| `fld_z` | the z-score of Q(L) within the field |
| `fld_within100` | other lineups within 100 display points |
| `fld_mean_q` | the field's mean Q(L) |
| `fld_top10_q` | the mean Q(L) of its ten best lineups |
| `fld_sd_q` | the spread of Q(L) in the field |
| `fld_spread14` | the first-to-fourth gap of the field |
| `cal_month` | calendar month |
| `cal_season_month` | months since September |
| `cal_dow` | day of the week |
| `cal_dom` | day of the month |
| `cal_same_date` | in-person tournaments played on the same date |
| `cal_season_day` | days since the season opened |
| `cal_home` | whether the team's town is the tournament's town |
| `ext_old_pred` | the old system's predictedPosition |
| `ext_gg_forecast` | chgk.gg's own forecast |
| `ext_gg_mu_before` | chgk.gg's team rating that morning |
| `ext_fun_forecast` | rating.chgk.fun's forecast |
| `ext_quest_forecast` | chgk.quest's forecast |

## 2. The leakage check

`uv run tools/features_check.py --a out/discovery/features.db --b
out/discovery/features_scrambled.db --date 2024-01-01`, three tests from the
blunt to the decisive. It passes all three.

Columns: 138 in all, 128 of them features.
No feature correlates with the finishing place above 0.90 within a field; the best of them is the model's own forecast, which is not a feature.

### Re-derived from strictly earlier rows

| outcome column | rows checked | disagreements | largest |
|---|---:|---:|---:|
| `ed_team_n` | 58487 | 0 | 0.00e+00 |
| `ed_team_resid` | 58487 | 0 | 0.00e+00 |
| `ed_team_tournaments` | 58487 | 0 | 0.00e+00 |
| `tm_best_place365` | 58487 | 0 | 0.00e+00 |
| `tm_surp365_mean` | 58487 | 0 | 0.00e+00 |
| `tm_surp365_n` | 58487 | 0 | 0.00e+00 |
| `tm_surp5_mean` | 58487 | 0 | 0.00e+00 |
| `tm_worst_place365` | 58487 | 0 | 0.00e+00 |

### The scramble test

Scrambled the first in-person field on or after 2024-01-01: tournament 10087 of 2024-01-06, 19 results, with 10518 results in the 391 fields that follow it.
Rows of the fields before it that moved: 0.
**No feature of the 19 results moved** when that field's own places and masks were permuted: not one of the 128 columns.
Columns that moved in the fields after it, as they must: 77 of 128.

**PASSED**: census ok, re-derivation ok, scramble ok.

The third is the one that matters. `fit --scramble DATE` permutes the finishing
places **and the answer masks** of the first in-person field on or after DATE.
Nothing a lineup can be judged by that morning has changed, so every feature of
that field must come out bit-identical — and all 128 do, over its 19 results.
That the same permutation moves 77 of the 128 columns in the 391 fields that
follow is what says the test has teeth rather than no signal.

## 3. The ceilings

| system | tune | @3 | tune, unweighted | @3 | test | @3 | test, unweighted | @3 | fields |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| round 3, the shipped forecast | 0.8963 | 0.7834 | 0.9014 | 0.7988 | 0.9084 | 0.8108 | 0.9070 | 0.8122 | 650 / 284 |
| the ranker, no external features | 0.8990 | 0.7825 | 0.9023 | 0.7959 | 0.9037 | 0.7993 | 0.9032 | 0.8030 | 650 / 284 |
| the ranker, with them | 0.8977 | 0.7829 | 0.9033 | 0.8013 | 0.9093 | 0.8070 | 0.9078 | 0.8086 | 650 / 284 |
| the ranker started from round 3 | 0.8982 | 0.7865 | 0.9026 | 0.8007 | 0.9038 | 0.8018 | 0.9032 | 0.8051 | 650 / 284 |
| the same, with the external features | 0.8980 | 0.7845 | 0.9034 | 0.8005 | 0.9058 | 0.8043 | 0.9047 | 0.8067 | 650 / 284 |
| rating.chgk.fun | 0.8893 | 0.7684 | 0.8963 | 0.7898 | 0.9044 | 0.8001 | 0.8988 | 0.7938 | 327 / 284 |
| chgk.quest | 0.8935 | 0.7747 | 0.8847 | 0.7674 | 0.9011 | 0.7914 | 0.8974 | 0.7883 | 646 / 282 |

Paired bootstrap over the fields both rows cover, 2000 resamples, weighted.

| a | b | window | NDCG | SE | NDCG@3 | SE | fields |
|---|---|---|---:|---:|---:|---:|---:|
| the ranker, no external features | round 3, the shipped forecast | tune | +0.0027 | 0.0018 | -0.0009 | 0.0042 | 650 |
| the ranker, no external features | round 3, the shipped forecast | test | -0.0047 | 0.0026 | -0.0115 | 0.0058 | 284 |
| the ranker, no external features | rating.chgk.fun | tune | +0.0142 | 0.0051 | +0.0278 | 0.0115 | 327 |
| the ranker, no external features | rating.chgk.fun | test | -0.0007 | 0.0039 | -0.0008 | 0.0087 | 284 |
| the ranker, no external features | chgk.quest | tune | +0.0060 | 0.0028 | +0.0086 | 0.0065 | 646 |
| the ranker, no external features | chgk.quest | test | +0.0027 | 0.0040 | +0.0082 | 0.0084 | 282 |
| the ranker, with them | round 3, the shipped forecast | tune | +0.0014 | 0.0019 | -0.0006 | 0.0050 | 650 |
| the ranker, with them | round 3, the shipped forecast | test | +0.0009 | 0.0017 | -0.0038 | 0.0047 | 284 |
| the ranker, with them | rating.chgk.fun | tune | +0.0155 | 0.0048 | +0.0339 | 0.0104 | 327 |
| the ranker, with them | rating.chgk.fun | test | +0.0049 | 0.0036 | +0.0068 | 0.0080 | 284 |
| the ranker, with them | chgk.quest | tune | +0.0047 | 0.0026 | +0.0090 | 0.0062 | 646 |
| the ranker, with them | chgk.quest | test | +0.0084 | 0.0039 | +0.0159 | 0.0091 | 282 |
| the ranker started from round 3 | round 3, the shipped forecast | tune | +0.0019 | 0.0011 | +0.0030 | 0.0025 | 650 |
| the ranker started from round 3 | round 3, the shipped forecast | test | -0.0046 | 0.0020 | -0.0090 | 0.0034 | 284 |
| the ranker started from round 3 | rating.chgk.fun | tune | +0.0141 | 0.0049 | +0.0333 | 0.0106 | 327 |
| the ranker started from round 3 | rating.chgk.fun | test | -0.0006 | 0.0036 | +0.0017 | 0.0080 | 284 |
| the ranker started from round 3 | chgk.quest | tune | +0.0052 | 0.0025 | +0.0126 | 0.0060 | 646 |
| the ranker started from round 3 | chgk.quest | test | +0.0027 | 0.0038 | +0.0106 | 0.0083 | 282 |
| the same, with the external features | round 3, the shipped forecast | tune | +0.0017 | 0.0011 | +0.0011 | 0.0025 | 650 |
| the same, with the external features | round 3, the shipped forecast | test | -0.0026 | 0.0015 | -0.0065 | 0.0028 | 284 |
| the same, with the external features | rating.chgk.fun | tune | +0.0128 | 0.0049 | +0.0290 | 0.0107 | 327 |
| the same, with the external features | rating.chgk.fun | test | +0.0014 | 0.0033 | +0.0042 | 0.0074 | 284 |
| the same, with the external features | chgk.quest | tune | +0.0050 | 0.0027 | +0.0106 | 0.0062 | 646 |
| the same, with the external features | chgk.quest | test | +0.0049 | 0.0040 | +0.0133 | 0.0085 | 282 |

Read across the first two rows. **The ranker gains 0.0027 on the window it was
tuned on and gives back 0.0047 on the window it was not**, and NDCG@3 — the part
of the metric that decides the podium — is negative on both. Starting the ranker
from round 3's own forecast, so that every tree it grows is a correction rather
than a reconstruction, tightens the standard error and tells the same story:
+0.0019 ± 0.0011 and −0.0046 ± 0.0020.

The one row that holds level on test is **the ranker with the external systems'
forecasts in hand**, +0.0009 ± 0.0017. That is the plan's diagnostic working as
intended: what a ranker can add to our own state is nothing, and what it can add
given three rival systems' published forecasts is one standard error. The
external columns may never be distilled, and the family ablation below says they
are not even worth their place.

The ranker's own seed wobbles by about 0.0005 between runs of the same
configuration, which is a third of the tune gain and a tenth of the test loss.

CatBoost YetiRank, the cross-check the plan asks for, was trained on the same
127 features against fold 0 and the test window: CatBoost YetiRank: {'fold0': 0.8942993174588285, 'fold0_round3': 0.8938000554932253, 'test': 0.9076854882479792, 'test3': 0.8085178645214787, 'n_features': 127, 'trees': 138}. Its fold-0 score is
0.8943 against round 3's 0.8938 on the same fields, and its test score 0.9077
against 0.9084. Two different boosters, two different ranking losses, the same
answer.

## 4. What the ranker leans on

Out-of-fold weighted NDCG of the model the importances are read off: 0.8977.

| # | feature | family | permutation | gain rank | SHAP rank |
|---:|---|---|---:|---:|---:|
| 1 | `fld_gap_fourth` | field context | +0.04849 | 1 | 1 |
| 2 | `fld_z` | field context | +0.02798 | 2 | 2 |
| 3 | `fld_rank_frac` | field context | +0.00931 | 4 | 4 |
| 4 | `ext_old_pred` | external systems | +0.00399 | 5 | 3 |
| 5 | `fld_rank_q` | field context | +0.00392 | 3 | 5 |
| 6 | `fld_gap_first` | field context | +0.00212 | 6 | 6 |
| 7 | `agg_phi_min` | player aggregates | +0.00204 | 21 | 15 |
| 8 | `agg_thetaphi_sd` | player aggregates | +0.00187 | 31 | 14 |
| 9 | `agg_phi_max` | player aggregates | +0.00180 | 12 | 16 |
| 10 | `ed_team_resid` | editor | +0.00180 | 18 | 56 |
| 11 | `shp_q_minus_qbar` | lineup shape | +0.00176 | 9 | 7 |
| 12 | `agg_vol_min` | player aggregates | +0.00176 | 24 | 23 |
| 13 | `agg_games_min` | player aggregates | +0.00171 | 46 | 43 |
| 14 | `agg_dtheta90_min` | player aggregates | +0.00164 | 19 | 55 |
| 15 | `agg_phi_mean` | player aggregates | +0.00163 | 11 | 12 |
| 16 | `tm_surp5_sd` | team state | +0.00163 | 10 | 24 |
| 17 | `tm_games` | team state | +0.00156 | 38 | 38 |
| 18 | `tm_best_place365` | team state | +0.00143 | 15 | 31 |
| 19 | `tm_surp5_mean` | team state | +0.00132 | 20 | 22 |
| 20 | `tm_surp365_mean` | team state | +0.00130 | 32 | 78 |
| 21 | `agg_dtheta365_min` | player aggregates | +0.00125 | 22 | 25 |
| 22 | `ed_player_gated` | editor | +0.00112 | 27 | 35 |
| 23 | `shp_size` | lineup shape | +0.00111 | 29 | 9 |
| 24 | `ext_quest_forecast` | external systems | +0.00109 | 8 | 10 |
| 25 | `ed_pairs_gated` | editor | +0.00105 | 58 | 41 |

Permutation is the drop in out-of-fold weighted NDCG when the column is shuffled
**within each tournament**, so the field context and the calendar stay what they
were and only that column's ordering role is broken. Gain and SHAP are ranks
over the same 132 columns.

The table is really two tables. The first six rows are worth +0.048, +0.028,
+0.009, +0.004, +0.004 and +0.002, and five of them — `fld_gap_fourth`,
`fld_z`, `fld_rank_frac`, `fld_rank_q`, `fld_gap_first` — **are monotone
transforms of `Q(L)` inside the field**. Ordering a field by any one of them is
ordering it by `Q(L)`, which is ordering it by round 3's forecast. They are not
a discovery; they are the change of variables a tree ensemble needs before it
can express a per-field ranking at all, because a regression function fitted
across fields cannot rank inside one from an absolute strength alone.

The sixth is `ext_old_pred`, the old system's `predictedPosition` — another
system's whole ranking, arriving as one column.

Everything from row seven down is between +0.0021 and +0.0010, and the paired SE
of the tune window is 0.0018. **After the restatements of `Q(L)`, nothing in 128
columns clears one standard error.** The things that do appear there are the
recognisable ones: the format effect `φ` at the extremes of the lineup, the
spread of `θ + φ` across it, the team's own run of surprises, the editor memory
already shipped and the team-level version of it, `Q(L) − Q̄`.

## 5. The family ablations

Each family dropped whole and the ranker retrained, five folds, out of fold.

| features | columns | tune NDCG | @3 | cost |
|---|---:|---:|---:|---:|
| all | 132 | 0.8977 | 0.7829 | - |
| minus lineup strength | 111 | 0.8980 | 0.7873 | +0.0004 |
| minus player aggregates | 78 | 0.8979 | 0.7849 | +0.0002 |
| minus lineup shape | 120 | 0.8980 | 0.7855 | +0.0004 |
| minus team state | 119 | 0.8986 | 0.7872 | +0.0009 |
| minus editor | 124 | 0.8985 | 0.7842 | +0.0009 |
| minus field context | 120 | 0.8886 | 0.7648 | -0.0090 |
| minus calendar and place | 125 | 0.8990 | 0.7851 | +0.0014 |
| minus external systems | 127 | 0.8990 | 0.7825 | +0.0013 |

**Every family except one can be dropped for free**, and six of the seven come
back very slightly *better* — dropping 54 player aggregates is +0.0002, dropping
21 lineup strengths is +0.0004, dropping the calendar is +0.0014 and dropping
the external systems is +0.0013. That is not redundancy between families so much
as one signal restated eight ways: `agg_theta_*` and `lin_*` and `fld_*` are the
same lineup seen three times, and a ranker handed all three overfits the seams.

The exception is the field context, −0.0090. Take away the lineup's position in
its own field and the ranker must rank by an absolute number again, and loses
four times more than any other family is worth.

## 6. The forward-selection curve

The best 1, 2, 3, 5, 8 and 12 features by permutation importance, external
columns excluded because the distillation may not use them, each retrained from
scratch on the same five folds.

| features | tune NDCG | @3 | test NDCG | @3 | the columns |
|---:|---:|---:|---:|---:|---|
| 1 | 0.8973 | 0.7846 | 0.9082 | 0.8080 | `fld_gap_fourth` |
| 2 | 0.8955 | 0.7816 | 0.9048 | 0.8034 | `fld_gap_fourth`, `fld_z` |
| 3 | 0.8948 | 0.7785 | 0.9076 | 0.8077 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac` |
| 5 | 0.8976 | 0.7845 | 0.9056 | 0.8042 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac`, `fld_rank_q`, `fld_gap_first` |
| 8 | 0.8969 | 0.7830 | 0.9074 | 0.8054 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac`, `fld_rank_q`, `fld_gap_first`, `agg_phi_min`, `agg_thetaphi_sd`, `agg_phi_max` |
| 12 | 0.8962 | 0.7796 | 0.9079 | 0.8065 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac`, `fld_rank_q`, `fld_gap_first`, `agg_phi_min`, `agg_thetaphi_sd`, `agg_phi_max`, `ed_team_resid`, `shp_q_minus_qbar`, `agg_vol_min`, `agg_games_min` |

**One feature is the whole curve.** `fld_gap_fourth` alone — how far this
lineup's `Q(L)` stands from the fourth best in its field — scores 0.8973 on tune
and **0.9082 on test, against round 3's 0.9084**. Adding the other eleven never
improves the tune window and never recovers the test window: twelve features
give 0.8962 and 0.9079.

That single column is round 3's forecast recentred on the fourth place of the
field, which is exactly the quantity round 2 found the loss concentrated in
(`out/q_ndcg/round2.md` section 1.4: NDCG runs from 0.847 to 0.936 with the
first-to-fourth gap). The ranker rediscovered the diagnosis and could not get
past it.

## 7. The linear ranker on the top eight

Pairwise logistic on 159542 pairs of the tune window, features standardised.

| feature | coefficient |
|---|---:|
| `fld_gap_fourth` | +0.968 |
| `fld_z` | +0.036 |
| `fld_rank_frac` | -0.688 |
| `fld_rank_q` | -0.354 |
| `fld_gap_first` | +0.828 |
| `agg_phi_min` | +0.060 |
| `agg_thetaphi_sd` | -0.136 |
| `agg_phi_max` | -0.060 |

tune 0.8945 @3 0.7797, test 0.9025 @3 0.7965
  linear minus round 3, the shipped forecast, tune: -0.0018 se 0.0015 (650)
  linear minus round 3, the shipped forecast, test: -0.0059 se 0.0020 (284)
  linear minus rating.chgk.fun, tune: +0.0116 se 0.0050 (327)
  linear minus rating.chgk.fun, test: -0.0019 se 0.0035 (284)

Standardised, so the coefficients are comparable. The five field-context columns
carry everything — and they carry it in the arrangement that just says `Q(L)`:
positive on the gaps to first and fourth, negative on the rank. The three
non-trivial ones are an order of magnitude smaller: `agg_thetaphi_sd` at −0.136,
and `agg_phi_min` and `agg_phi_max` at ±0.060, the second pair cancelling.

The linear model **loses to round 3 on both windows**, −0.0018 ± 0.0015 on tune
and −0.0059 ± 0.0020 on test. So the plan's target — the linear model's number —
is below the plan's ceiling, and both are below what is already shipped.

## 8. The distillation

Of the top eight, five are `Q(L)` within its field and are therefore already the
shipped forecast exactly: there is no constant to add, because ranking by them
*is* ranking by `Q(L)`. Three are not, and two new switches were added to carry
them, both forecast-only like the editor memory of DESIGN 4.8 and both shipping
at zero.

| feature | rank | how it enters the simple model |
|---|---:|---|
| `fld_gap_fourth` | 1 | Already shipped. `Q(L)` less the field's fourth `Q(L)` orders a field exactly as `Q(L)` does; no constant. |
| `fld_z` | 2 | Already shipped, the same way: a per-field affine map of `Q(L)`. |
| `fld_rank_frac` | 3 | Already shipped: the forecast place itself, divided by the field size. |
| `fld_rank_q` | 5 | Already shipped: the forecast place. |
| `fld_gap_first` | 6 | Already shipped: `Q(L)` less the field's best. |
| `agg_phi_min` | 7 | **New.** `team.s2.phi_min`: the forecast gains that many display points per unit of the **smallest** `φ` at the table. The lineup strength already reads every player at `θ + φ`, so the lineup's mean `φ` is in and its weakest member's is not. |
| `agg_thetaphi_sd` | 8 | **New.** `team.s2.phi_spread`: display points per unit of the spread of `θ + φ` across the lineup — a lopsided table against a balanced one, which `τ` and `question.m_fixed` reach only indirectly. The linear ranker wants it negative. |
| `agg_phi_max` | 9 | Covered by `phi_min` with the opposite sign; the linear coefficients are +0.060 and −0.060 and cancel. |

Run through the round-3 harness on the tune window,
`uv run tools/q_ndcg_sweep.py --dir out/discovery/sweep --jobs 2
out/discovery/sweep/batch/distil.txt`, 18 fits against the same baseline. `d` is
the paired difference on the tune window with its bootstrap standard error.

| switch | tune NDCG | @3 | test NDCG | @3 | d tune | SE |
|---|---:|---:|---:|---:|---:|---:|
| `team.s2.phi_min=100.0` | 0.8964 | 0.7834 | 0.9073 | 0.8067 | 0.0001 | 0.0004 |
| `team.s2.phi_min=250.0` | 0.8963 | 0.7827 | 0.9063 | 0.8028 | -0.0000 | 0.0008 |
| `team.s2.phi_min=500.0` | 0.8955 | 0.7807 | 0.9048 | 0.8014 | -0.0008 | 0.0009 |
| `team.s2.phi_min=1000.0` | 0.8927 | 0.7760 | 0.9020 | 0.7954 | -0.0036 | 0.0014 |
| `team.s2.phi_min=-250.0` | 0.8948 | 0.7810 | 0.9092 | 0.8105 | -0.0015 | 0.0008 |
| `team.s2.phi_min=-500.0` | 0.8954 | 0.7806 | 0.9083 | 0.8085 | -0.0009 | 0.0012 |
| `team.s2.phi_spread=-250.0` | 0.8949 | 0.7780 | 0.9072 | 0.8093 | -0.0014 | 0.0008 |
| `team.s2.phi_spread=-500.0` | 0.8942 | 0.7765 | 0.9062 | 0.8052 | -0.0021 | 0.0013 |
| `team.s2.phi_spread=-1000.0` | 0.8936 | 0.7739 | 0.9029 | 0.7976 | -0.0027 | 0.0018 |
| `team.s2.phi_spread=250.0` | 0.8944 | 0.7799 | 0.9061 | 0.7999 | -0.0019 | 0.0012 |
| `team.s2.phi_spread=500.0` | 0.8918 | 0.7740 | 0.9051 | 0.7924 | -0.0045 | 0.0016 |
| `team.s2.tau=0.6` | 0.8948 | 0.7797 | 0.9069 | 0.8068 | -0.0015 | 0.0008 |
| `team.s2.tau=0.65` | 0.8955 | 0.7818 | 0.9072 | 0.8072 | -0.0008 | 0.0006 |
| `team.s2.tau=0.75` | 0.8951 | 0.7796 | 0.9075 | 0.8081 | -0.0012 | 0.0008 |
| `team.s2.tau=0.8` | 0.8948 | 0.7773 | 0.9069 | 0.8075 | -0.0015 | 0.0010 |
| `team.s2.sigma_ip=0.5` | 0.8966 | 0.7832 | 0.9081 | 0.8096 | 0.0003 | 0.0003 |
| `team.s2.sigma_ip=2.0` | 0.8954 | 0.7813 | 0.9083 | 0.8117 | -0.0009 | 0.0007 |
| `team.s2.phi_min=250.0 team.s2.phi_spread=-500.0` | 0.8939 | 0.7756 | 0.9050 | 0.8029 | -0.0024 | 0.0013 |

**Not one of them wins.** `phi_min` is +0.0001 ± 0.0004 at 100 points and falls
away monotonically in both directions — −0.0036 at 1000 and −0.0015 at −250 —
which is the shape of a constant whose best value is zero. `phi_spread` loses at
every value and both signs, −0.0014 at −250 and −0.0045 at +500, and the two
together are −0.0024. The temperature grid is the same statement through the
knob that already exists: `τ = 0.7` beats 0.6, 0.65, 0.75 and 0.8, so the
lopsidedness of a lineup is already priced. `sigma_ip`, which is what the
least-measured player at the table (`agg_games_min`, `agg_vol_min`, ranks 12 and
13) would enter through, is +0.0003 ± 0.0003 at 0.5 and negative at 2.

The ranker said these three columns were worth about +0.002 each in permutation
and the harness says they are worth nothing. Both statements are consistent with
the same reading: +0.002 is the noise floor of this metric on this window.

## 9. What this round says, and what it could not do

- **The ceiling is round 3.** Given every feature the pipeline can compute on
  the morning of a tournament, two boosting implementations and two ranking
  losses, nothing beats the shipped forecast out of sample. The best test-window
  number any ranker produced without the rivals' forecasts is 0.9082, from one
  feature, against 0.9084 shipped.
- **The remaining loss is resolution, not a missing feature.** DESIGN 7.4
  already said the forecast is calibrated in every direction the state can see;
  this round says the same thing from the other end, by failing to find a
  direction with 128 of them and a model free to combine them any way it likes.
- **A fourth round of levers keyed to lineup properties is not worth running.**
  Round 3 said this from seven residual checks; this says it from a wide search.
  If the margin over rating.chgk.fun and chgk.quest is to reach three standard
  errors, it will not be by ordering a field better from the same state.
- **The external-system diagnostic came back nearly empty too.** Handed three
  rival forecasts the ranker gains +0.0009 ± 0.0017 on the test window over
  round 3 — half a standard error — and the family ablation says dropping those
  columns *improves* the out-of-fold tune score by 0.0013. There is very little
  in those systems that we do not have.
- **Not done: a per-question feature table.** Every column here is a property of
  a lineup, a field or a calendar, and none is a property of the package. The
  difficulty sample exists (`src/difficulty.rs`) and round 3 measured the
  difficulty-aware forecast and lost, so this was not repeated, but a table with
  one row per question is a different object and this plan did not build it.
- **Not done: `ed_team_resid` as a switch.** It is 10th by permutation, worth
  +0.0018, and it is a team-level editor memory rather than the player-level one
  that ships. It is not in the top eight, so by the plan's own rule it was not
  distilled; it is the one column in the table with a mechanism behind it that
  has not been swept.
- **The two new switches are dead weight in the config.** `phi_min` and
  `phi_spread` were added to make the distillation measurable and both lost.
  They ship at zero, where the fit is bit-identical, and the next round should
  either use them or take them out.
- **A five-fold split of tune is not causal.** A model predicting one fifth has
  seen tune fields that come after it in time. That is the standard arrangement
  for feature discovery and it inflates the tune column rather than the test
  one, which is the column the conclusion rests on.
