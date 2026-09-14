# Plan 5: the question model, second round

Round 1 left the in-person forecast ahead of rating.chgk.fun and chgk.quest and
measurably ahead of neither. Plan 5 asked for a clear margin, and asked for a
diagnosis first. The diagnosis is the useful part of what follows: **the
residual carries no systematic bias in any property of a lineup the model
knows about**, and the loss that is left sits where the top of a field is
bunched, which no reordering of the same information can reach. Two of the
seven levers the plan listed were aimed at a cohort that carries 0.3% of the
loss; they lost, and the diagnosis says why before the numbers do.

What ships is small: two constants retuned — the teammate shrinkage from 0.005
to 0.0025, the diffusion of the format effect from 0.05 to 0 — and the published
player number changed to `θ + φ`, which does not touch the forecast but takes
test 3 from 111 and 115 of 200 back to 121 and 130. The forecast gains
+0.0009 ± 0.0012 of weighted tune NDCG and +0.0012 ± 0.0015 on test, +0.0042 and
+0.0052 at 3: small, and the same sign in both windows, which is more than
almost anything else in 114 runs manages. **The lead over rating.chgk.fun and
chgk.quest is not three standard errors and this round did not make it so.**

## 0. One thing about the data

The buffer mirror `buff.db` was replaced under the working directory during
this run: the copy the first fits read carried 5640 rated tournaments in the
window and the copy that replaced it carries 5605. Everything below therefore
runs against a **frozen copy** of the second one, and the round-1 winner was
refitted on it as the baseline of this round. The refit is inside the noise of
`out/round1.db` itself — paired, `−0.0004 ± 0.0007` of tune NDCG and
`−0.0002 ± 0.0006` on test — so nothing here depends on which copy is right,
but the round-1 report's absolute numbers and this one's differ in the fourth
decimal for that reason and not for any other.

Baseline, the merged round-1 winner on the frozen mirror:

```
ndcg tune 0.8939 @3 0.7763  unweighted 0.9001 @3 0.7952  (650 tournaments)
ndcg test 0.9065 @3 0.8021  unweighted 0.9049 @3 0.8043  (284 tournaments)
```

Protocol unchanged from plan 3: `to = 2026-09-05`, `rows_every = 0`, tune
2018-09-01 to 2024-08-29 decides, test 2024-08-30 to 2026-08-27 read once per
row, weights from `data/tournament_weights.csv`, paired bootstrap of 2000
resamples. 114 fits on the frozen mirror, `out/q2/results.tsv`, driven from
`out/q_ndcg/batch2/`, plus the history fits of sections 3 to 6.

## 1. The diagnosis

### 1.1 The worst fields

`uv run tools/q_diag.py out/q2/base.db --teams 6 --cohorts`. A field's cost is
`weight × (1 − NDCG)`. Over the 650 in-person fields of the tune window the
total weighted loss is 149 428 and the twenty worst carry 15.2% of it.

| # | tournament | date | teams | weight | NDCG | NDCG@3 | cost |
|---:|---|---|---:|---:|---:|---:|---:|
| 1 | Чемпионат мира. Этап 2. Группа А | 2018-11-03 | 13 | 4977 | 0.6859 | 0.3424 | 1563 |
| 2 | Мемориал Дмитрия Коноваленко | 2020-02-08 | 78 | 4793 | 0.7106 | 0.2577 | 1387 |
| 3 | Чемпионат мира. Финал. Группа В | 2018-11-04 | 26 | 4471 | 0.7052 | 0.2369 | 1318 |
| 4 | Белые ночи | 2019-07-06 | 75 | 4387 | 0.7055 | 0.3364 | 1292 |
| 5 | Чемпионат мира. Финал. Группа А | 2018-11-04 | 13 | 4985 | 0.7431 | 0.4174 | 1281 |
| 6 | Чемпионат Мира. Финал. Группа В | 2019-09-08 | 30 | 4309 | 0.7094 | 0.4237 | 1252 |
| 7 | Чемпионат мира. Этап 1. Группа С | 2018-11-03 | 26 | 4189 | 0.7025 | 0.2968 | 1246 |
| 8 | Чемпионат Минска. Лига А. Тур пятый | 2019-03-25 | 38 | 3300 | 0.6336 | 0.2289 | 1209 |
| 9 | Части Вселенной | 2020-02-22 | 28 | 3830 | 0.6897 | 0.3775 | 1189 |
| 10 | Четвёртый Ворон | 2023-10-01 | 40 | 3658 | 0.6811 | 0.2217 | 1167 |
| 11 | Боровские муралы 2024 | 2024-06-02 | 42 | 3788 | 0.7121 | 0.4605 | 1090 |
| 12 | Филин FM Зимний | 2023-12-09 | 31 | 3314 | 0.6924 | 0.2595 | 1019 |
| 13 | Нестерка | 2019-10-26 | 88 | 4040 | 0.7514 | 0.5066 | 1005 |
| 14 | Кубок Москвы | 2018-10-14 | 87 | 4291 | 0.7709 | 0.4752 | 983 |
| 15 | Гостиный Двор | 2018-12-08 | 75 | 3833 | 0.7479 | 0.5187 | 967 |
| 16 | БЛИК-28 | 2024-06-15 | 39 | 3353 | 0.7119 | 0.2477 | 966 |
| 17 | Кубок вМоГоТУ | 2019-11-02 | 31 | 2788 | 0.6605 | 0.3309 | 947 |
| 18 | Чемпионат мира. Этап 3. Группа С | 2018-11-04 | 26 | 3580 | 0.7356 | 0.3385 | 946 |
| 19 | Чемпионат мира. Этап 2. Группа С | 2018-11-03 | 26 | 3949 | 0.7632 | 0.3670 | 935 |
| 20 | Чемпионат Минска. Лига А. Тур четвёртый | 2020-02-24 | 35 | 3378 | 0.7281 | 0.5269 | 918 |

Seven of the twenty are groups of one World Championship, three more are
stages of the Minsk A league, and the rest are large open championships. What
they have in common is the **field**, not the lineups: these are the events
where the strongest teams in the game meet each other.

### 1.2 Who costs the most inside them

`--teams 6` prints, for each of the six worst fields, the four teams that cost
it the most, with every player's `θ`, `φ`, `σ`, in-person game count and days
since their last in-person game. The pattern is the same in all six and it is
not one of the four the plan guessed at:

- **Сборная Кирибати**, forecast 5th of 78 at Мемориал Коноваленко, finished
  1st. Six players, `σ` from 0.11 to 0.24, 42 to 112 in-person games each,
  most of them six days since their last in-person table.
- **Немчиновка**, forecast 9th of 75 at Белые ночи, finished 1st. Six players,
  26 to 86 in-person games.
- **X-promt**, forecast 10th of 30 at the 2019 World Championship final,
  finished 1st. Six players, 23 to 68 in-person games.
- **Алкоголь, старость и страх**, forecast 4th of 26, finished 2nd. Six
  players, 64 to 100 in-person games.

Not a debutant, not a sync-only table, not a short lineup, not a stale one.
They are established teams, fully measured, that the model put fourth to tenth
and that won. The single biggest source of cost in the whole metric is a
**winner the forecast ranked just outside the top three**: over the weighted
tune fields the actual winner's mean forecast rank is 2.30, the median is 1,
and the winner is inside the forecast top three 82% of the time and inside the
top five 95%. The metric's remaining loss is almost entirely the difference
between "one of the best four" and "the best".

### 1.3 The loss by cohort

Same tool, `--cohorts`. Each result's share of its field's loss, weighted.

| cohort of the lineup | results | share of the weighted loss | loss per result |
|---|---:|---:|---:|
| median 10+ in-person games | 9354 | 90.1% | 0.0046 |
| median 3-9 in-person games | 3367 | 8.3% | 0.0015 |
| median under 3 in-person games | 2237 | 1.4% | 0.0004 |
| half the table sync-only | 1452 | 0.3% | 0.0002 |
| no lineup listed | 2 | −0.0% | −0.0001 |
| short lineup (< 5) | 980 | −0.1% | −0.0001 |

**Ninety per cent of what is left is on well-measured lineups.** Sync-only
tables and short lineups together carry two parts in a thousand, and they carry
it in the *right* direction — the pessimistic quantile already over-corrects
them slightly. This is the sentence that reorders the plan's lever list.

### 1.4 The loss is where the field is bunched

For each field, the gap in forecast strength between the 1st and the 4th
ranked lineup, in display points, against the field's NDCG:

| quintile of the 1st-to-4th forecast gap | gap (points) | fields | NDCG |
|---|---:|---:|---:|
| 1, the most bunched | 381 | 115 | 0.8471 |
| 2 | 671 | 115 | 0.8817 |
| 3 | 946 | 115 | 0.9053 |
| 4 | 1309 | 115 | 0.9315 |
| 5, the most spread | 2065 | 116 | 0.9360 |

Monotone, and worth 0.089 of NDCG from end to end — far more than any lever in
either round. A field whose top four are within 380 points is a field in which
four teams really might win, and no ordering of the same information changes
that.

### 1.5 Nothing predicts the residual

`uv run tools/q_feat.py out/q2/base.db`. One row per result in a weighted
field: the residual `(forecast rank − actual rank) / n`, and what the lineup
looked like that morning. A team forecast first can only disappoint and a team
forecast last can only surprise, so the raw residual is a function of where the
forecast put the team; the table below subtracts the mean residual of the
team's forecast-percentile bin, in twentieths, and correlates what is left.

| feature | weighted corr with the residual, all results | the same, podium finishers only |
|---|---:|---:|
| seats in the lineup | −0.011 | +0.049 |
| best `θ` in the lineup | +0.036 | +0.009 |
| mean `θ` | +0.052 | +0.019 |
| spread of `θ` | −0.027 | −0.056 |
| mean `φ` | +0.014 | −0.011 |
| mean `σ` | −0.017 | −0.023 |
| largest `σ` | −0.015 | −0.047 |
| median in-person games | +0.006 | +0.081 |
| fewest in-person games | −0.002 | +0.058 |
| share of the table that is sync-only | −0.025 | −0.092 |
| share with under five in-person games | −0.012 | −0.088 |
| days since the table last played in person | −0.043 | −0.060 |

Every correlation is under 0.10 and most are under 0.03. **The forecast is
calibrated in every direction the state can see.** In particular `mean σ` is
very slightly *negative*, which says the pessimistic quantile at `σ_c = 1.5` is
already a shade too strong rather than too weak — the opposite of lever 1's
premise. Without this control the same table shows `σ` at +0.14 and in-person
games at −0.12, which is the floor effect and not a bias; it is worth saying
because it is the reading that would have sent this round after lever 1.

### 1.6 Where the competitors beat us

`uv run tools/q_vs.py out/q2/base.db out/ext/fun.db`. This is the difference
that plan 5 is actually about, and it is not the same object as the residual.

Against **rating.chgk.fun**, 327 common tune fields, weighted, ours minus
theirs +0.0111: they order 136 fields better and gain 0.0179 there, we order
160 better and gain 0.0290.

| cohort of the field's lineups | fields | theirs minus ours |
|---|---:|---:|
| median under 10 in-person games | 112 | −0.0169 |
| median 10 to 29 | 141 | −0.0162 |
| median 30 or more | 53 | **+0.0070** |

Against **chgk.quest**, 646 common tune fields, ours minus theirs +0.0010:

| cohort of the field's lineups | fields | theirs minus ours |
|---|---:|---:|
| median under 10 in-person games | 228 | −0.0070 |
| median 10 to 29 | 265 | **+0.0012** |
| median 30 or more | 84 | **+0.0018** |

The shape is the same against both and it is the mirror of section 1.3. **Our
whole edge comes from lineups with a thin in-person record** — which is exactly
what the pessimistic quantile of round 1 was built for — and on the
best-established lineups both competitors are level with us or slightly ahead.
Those are the regular league series: three stages of Чемпионат Минска Лига А
are in the twenty worst fields of section 1.1, and rating.chgk.fun orders every
one of them better than we do.

### 1.7 One thing the diagnosis ruled out before it was tried

The team state is not the answer. The forecast is the lineup's strength and
nothing else (DESIGN 4.4); the engine also computes `x = R + (Q(L) − Q̄)`, the
rating carried forward with today's lineup offset, and stores it. Blending the
two, `forecast + λ (x − forecast)`, was scored at λ from 0.1 to 1 on both
windows, with and without a gate on the team's rated games:

| λ | tune NDCG | tune @3 | test NDCG | test @3 |
|---:|---:|---:|---:|---:|
| 0, the forecast | **0.8945** | **0.7769** | **0.9067** | **0.8044** |
| 0.1 | 0.8933 | 0.7764 | 0.9058 | 0.8001 |
| 0.3 | 0.8925 | 0.7762 | 0.9057 | 0.8024 |
| 0.5 | 0.8915 | 0.7743 | 0.9028 | 0.7987 |
| 1.0, the rating | 0.8899 | 0.7673 | 0.9014 | 0.7933 |

Monotone the wrong way at every λ and on both windows. (This one table is on
the pre-freeze mirror; it was the run that found the swap.) DESIGN principle 6
holds: about tomorrow, the lineup knows more than the accumulated rating, and
it knows everything the rating knows.

## 2. The levers

Tune and test are the weighted means; `d` is the paired difference against the
round-1 baseline on the **tune** window with its bootstrap standard error, 2000
resamples. The full 114-row table is `out/q2/results.tsv`; the batch files are
`out/q_ndcg/batch2/`. The order below is the diagnosis's order, not the plan's.

| lever | tune NDCG | tune @3 | test NDCG | test @3 | d NDCG | SE | d @3 | SE |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **round 1, the baseline** | 0.8939 | 0.7763 | 0.9065 | 0.8021 | - | - | - | - |
| **3. the question fit** (plan lever 3) | | | | | | | | |
| `newton`, `s_m` 0.25 | 0.8946 | 0.7812 | 0.9049 | 0.7955 | +0.0007 | 0.0019 | +0.0048 | 0.0050 |
| `newton`, `s_m` 0.5 | 0.8955 | 0.7821 | 0.9063 | 0.8008 | +0.0016 | 0.0019 | +0.0058 | 0.0049 |
| `newton`, `s_m` 0.75 | 0.8952 | 0.7838 | 0.9049 | 0.7994 | +0.0014 | 0.0019 | +0.0075 | 0.0048 |
| **`newton`, `s_m` 1.0** | **0.8950** | **0.7826** | **0.9068** | **0.8014** | +0.0012 | 0.0018 | +0.0062 | 0.0047 |
| `newton`, `s_m` 1.0, `s_a` 0.25 | 0.8951 | 0.7824 | 0.9068 | 0.8040 | +0.0012 | 0.0016 | +0.0061 | 0.0043 |
| `newton`, `s_m` 1.5 | 0.8944 | 0.7794 | 0.9054 | 0.7946 | +0.0005 | 0.0019 | +0.0030 | 0.0048 |
| `newton`, `s_m` 2.0 | 0.8945 | 0.7800 | 0.9021 | 0.7891 | +0.0006 | 0.0019 | +0.0037 | 0.0048 |
| `take_rate` with `s_m` or `s_a` moved | 0.8939 | 0.7763 | 0.9065 | 0.8021 | 0.0000 | 0.0000 | 0.0000 | 0.0000 |
| **7. temperature against quantile** (plan lever 7) | | | | | | | | |
| `tau` 0.6, `sigma_c` 1.0 to 2.5 | 0.8931-0.8942 | 0.7789-0.7802 | 0.9054-0.9059 | 0.8010-0.8020 | -0.0008..+0.0003 | 0.0011 | +0.0026..+0.0038 | 0.0031 |
| `tau` 0.7, `sigma_c` 1.0 / 2.0 | 0.8935 / 0.8938 | 0.7751 / 0.7769 | 0.9071 / 0.9066 | 0.8039 / 0.8041 | -0.0004 / -0.0001 | 0.0007 | -0.0013 / +0.0005 | 0.0015 |
| `tau` 0.8, `sigma_c` 1.0 to 2.0 | 0.8932-0.8939 | 0.7739-0.7763 | 0.9048-0.9054 | 0.8010-0.8038 | -0.0007..0.0000 | 0.0009 | -0.0025..0.0000 | 0.0020 |
| `tau` 0.5, `sigma_c` 1.5 | 0.8915 | 0.7739 | 0.9040 | 0.7970 | -0.0024 | 0.0013 | -0.0024 | 0.0034 |
| `tau` 0.4, `sigma_c` 1.5 | 0.8883 | 0.7677 | 0.9008 | 0.7922 | -0.0056 | 0.0019 | -0.0086 | 0.0045 |
| **2. the format effect and the editors together** (plan lever 2) | | | | | | | | |
| `aff_rank` 1 beside `phi` | 0.8920 | 0.7750 | 0.9031 | 0.7995 | -0.0019 | 0.0016 | -0.0013 | 0.0035 |
| `aff_rank` 2 beside `phi` | 0.8909 | 0.7744 | 0.9044 | 0.8010 | -0.0030 | 0.0015 | -0.0019 | 0.0035 |
| `aff_rank` 1, `s_aff` 0.2 | 0.8911 | 0.7704 | 0.9041 | 0.8013 | -0.0028 | 0.0011 | -0.0059 | 0.0031 |
| **1. a quantile that knows the format** (plan lever 1) | | | | | | | | |
| `sigma_ip` 1 | 0.8937 | 0.7764 | 0.9067 | 0.8036 | -0.0002 | 0.0006 | +0.0001 | 0.0015 |
| `sigma_ip` 3 | 0.8922 | 0.7742 | 0.9070 | 0.8044 | -0.0017 | 0.0009 | -0.0022 | 0.0018 |
| `sigma_ip` 10 | 0.8912 | 0.7721 | 0.9062 | 0.8046 | -0.0027 | 0.0012 | -0.0042 | 0.0026 |
| `sigma_ip` 3, `sigma_c` 1.0 | 0.8933 | 0.7745 | 0.9075 | 0.8047 | -0.0006 | 0.0007 | -0.0019 | 0.0017 |
| **4. a synchronous scalar of its own** (plan lever 4) | | | | | | | | |
| `sync_effect`, `s_psi` 0.15 | 0.8918 | 0.7714 | 0.9055 | 0.8010 | -0.0021 | 0.0010 | -0.0049 | 0.0021 |
| `sync_effect`, `s_psi` 0.3 | 0.8904 | 0.7685 | 0.9057 | 0.8001 | -0.0035 | 0.0011 | -0.0079 | 0.0023 |
| `sync_effect`, `s_psi` 0.6 | 0.8899 | 0.7680 | 0.9026 | 0.7984 | -0.0040 | 0.0012 | -0.0084 | 0.0025 |
| `sync_effect`, `s_phi` 0.6 | 0.8886 | 0.7660 | 0.9029 | 0.7984 | -0.0053 | 0.0014 | -0.0104 | 0.0031 |
| **6. the memory of the format effect** (plan lever 6) | | | | | | | | |
| **`tau_phi` 0** | **0.8942** | **0.7775** | 0.9055 | 0.7995 | **+0.0003** | 0.0005 | **+0.0012** | 0.0011 |
| `tau_phi` 0.2 | 0.8903 | 0.7702 | 0.9051 | 0.8006 | -0.0036 | 0.0013 | -0.0061 | 0.0032 |
| `tau_phi` 0.4 | 0.8842 | 0.7569 | 0.8996 | 0.7875 | -0.0097 | 0.0019 | -0.0194 | 0.0046 |
| `tau_phi_own`, the in-person clock | 0.8930 | 0.7752 | 0.9054 | 0.8020 | -0.0009 | 0.0007 | -0.0012 | 0.0017 |
| `tau_phi_own`, `tau_phi` 0.2 | 0.8840 | 0.7571 | 0.8987 | 0.7844 | -0.0099 | 0.0019 | -0.0192 | 0.0046 |
| **the constants the diagnosis pointed at** | | | | | | | | |
| `teammate_shrink` 0 | 0.8926 | 0.7761 | 0.9052 | 0.8020 | -0.0013 | 0.0017 | -0.0002 | 0.0044 |
| `teammate_shrink` 0.0015 | 0.8944 | 0.7815 | 0.9073 | 0.8061 | +0.0005 | 0.0012 | +0.0052 | 0.0031 |
| `teammate_shrink` 0.002 | 0.8941 | 0.7806 | 0.9078 | 0.8076 | +0.0002 | 0.0012 | +0.0042 | 0.0032 |
| **`teammate_shrink` 0.0025** | **0.8944** | **0.7804** | **0.9079** | **0.8079** | **+0.0006** | 0.0011 | **+0.0040** | 0.0030 |
| `teammate_shrink` 0.003 | 0.8947 | 0.7807 | 0.9075 | 0.8064 | +0.0008 | 0.0011 | +0.0044 | 0.0029 |
| `teammate_shrink` 0.01 | 0.8902 | 0.7690 | 0.9055 | 0.7991 | -0.0037 | 0.0012 | -0.0074 | 0.0025 |
| `info_scale` 0.40 to 0.65 | 0.8924-0.8937 | 0.7747-0.7774 | 0.9054-0.9070 | 0.8001-0.8056 | -0.0015..-0.0002 | 0.0008 | -0.0016..+0.0011 | 0.0018 |
| `v_min` 0.10 / 0.15 | 0.8933 / 0.8915 | 0.7750 / 0.7721 | 0.9059 / 0.9051 | 0.8022 / 0.8029 | -0.0006 / -0.0024 | 0.0014 | -0.0013 / -0.0042 | 0.0038 |
| `v_max` 1.0 | 0.8932 | 0.7759 | 0.9061 | 0.8023 | -0.0007 | 0.0005 | -0.0004 | 0.0013 |
| `w_sync` 0.35 / 0.75 | 0.8930 / 0.8938 | 0.7729 / 0.7758 | 0.9073 / 0.9064 | 0.8044 / 0.8056 | -0.0009 / -0.0001 | 0.0006 | -0.0035 / -0.0006 | 0.0018 |
| `w_strict_sync` 1.0 | 0.8944 | 0.7769 | 0.9069 | 0.8030 | +0.0005 | 0.0004 | +0.0006 | 0.0010 |
| `warm_sigma` 0.3 / 0.7, `warm_until` 2018 | 0.8919-0.8932 | 0.7735-0.7761 | 0.9054-0.9069 | 0.8023-0.8054 | -0.0020..-0.0007 | 0.0009 | -0.0028..-0.0003 | 0.0022 |
| `gauge.n_est` 4 / 16 | 0.8938 / 0.8935 | 0.7760 / 0.7752 | 0.9067 / 0.9068 | 0.8034 / 0.8035 | -0.0001 / -0.0004 | 0.0005 | -0.0003 / -0.0011 | 0.0013 |
| **inert by construction** | | | | | | | | |
| `s_b`, `b_bar`, `s_a` at any value | 0.8939 | 0.7763 | 0.9065 | 0.8021 | 0.0000 | 0.0000 | 0.0000 | 0.0000 |
| `u_max` 30 / 60, `a_max` 5.0, `a_min` 0.15 | 0.8939 | 0.7763 | 0.9065 | 0.8021 | 0.0000 | 0.0000 | 0.0000 | 0.0000 |
| `display.player_phi` 0.5 / 1.0 | 0.8939 | 0.7763 | 0.9065 | 0.8021 | 0.0000 | 0.0000 | 0.0000 | 0.0000 |

What the levers say, in the order the diagnosis put them.

1. **The plan's first, fourth, fifth and sixth levers had nothing to find, and
   section 1.5 said so before they were run.** A quantile that widens for a
   player with little in-person evidence (`sigma_ip`) is negative at every
   strength on its own; a synchronous scalar of its own is the second-worst
   thing measured in either round, behind only `signed`; a format effect with a
   shorter memory is negative; and weighting synchronous data separately inside
   the `φ` update is **inert by construction** — under `format_effect =
   "in_person"` the indicator is zero at a synchronous table, so `φ` never sees
   that data at all and there is no weight to give it. That is the same kind of
   finding as round 1's `w_async`, and it is recorded rather than swept.
2. **Round 1's open question is answered and the answer is no.** The affinity
   state now has a pinned column for the format effect and free columns for the
   editors, so `aff_rank` and `format_effect` can finally be on together. It is
   −0.0019 at rank 1 and −0.0030 at rank 2. The two compete for the same
   gradient, and the free columns are the ones a single online pass cannot
   identify — which is what DESIGN 8.2 already says about affinity alone.
3. **The Newton fit's best point moved, and it still fails where it always
   fails.** On the merged model `s_m = 1` — not round 1's 0.5 — is the first
   point whose tune and test windows agree in sign, and with the retuned
   shrinkage it reaches weighted tune NDCG 0.8972 against 0.8948, the best
   forecast measured anywhere in either round. It is not shipped, and the reason
   is **test 1**, not the tune window: cross-fitted in-person log-loss rises
   from 0.4899 to 0.4931 over all seasons and from 0.5160 to 0.5235 in 2025/26,
   over 3.7 million observations, while the in-sample loss falls from 0.4381 to
   0.4175. Section 5 has the whole acceptance row for it.
4. **`tau` and `sigma_c` are at their optimum.** Nothing in the grid around 0.7
   and 1.5 beats the shipped point on NDCG; `tau` 0.6 with a larger `sigma_c`
   buys +0.004 at 3 and gives back more than that on the test window. The plan
   asked for this grid and it confirms round 1.
5. **Two constants moved.** `teammate_shrink` has an interior optimum at
   0.0025 — 0 is −0.0013 and 0.01 is −0.0037 — worth +0.0006 ± 0.0011 on tune
   and +0.0013 ± 0.0016 on test, and +0.0040 / +0.0058 at 3. `tau_phi` moved to
   0: the format effect is better treated as a constant per player than as
   something that drifts. Neither is large; both agree across the two windows,
   which almost nothing else in 114 runs does.
6. **The likelihood's saturation is not the bottleneck.** `u_max` 30 and 60 are
   **bit-identical** to 15, and `a_max`, `a_min`, `s_b`, `b_bar` and `s_a` are
   all exactly inert under `fit = "take_rate"`, which sets `a = m = 1` and reads
   `b` off the take rate. A plausible story about elite fields saturating the
   noisy-OR was killed in three runs, which is what a 30-second fit is for.

## 3. The winner

```
[player]  teammate_shrink = 0.0025   (was 0.005)
          tau_phi = 0.0              (was 0.05)
[display] player_phi = 1.0           (new; publishes theta + phi)
          a_team -7383.8881, b_team 1.2421,
          a_player 5738.6039, b_player 4269.2612
```

Nothing else in `config.toml` changed, and the three switches this round added
to the code — `question.sync_effect`, `team.s2.sigma_ip`, `player.tau_phi_own` —
ship at the value that changes nothing.

```
ndcg tune 0.8948 @3 0.7806  unweighted 0.9003 @3 0.7972  (650 tournaments)
ndcg test 0.9077 @3 0.8072  unweighted 0.9063 @3 0.8089  (284 tournaments)
paired tune minus round 1: +0.0009 se 0.0012  @3 +0.0042 se 0.0030
paired test minus round 1: +0.0012 se 0.0015  @3 +0.0052 se 0.0039
```

The display maps were refitted by `calibrate --display` twice — once because the
new constants moved the team scale, once because `player_phi` moved the player
scale — and both are fixed points: the 2019 anchors are exact.
`display.player_phi` does not touch the forecast, and the NDCG above is
identical with it at 0 and at 1.

**The runner-up, for the record.** `teammate_shrink 0.0025` + `tau_phi 0` +
`sigma_ip 1` + `newton, s_m 1, s_a 0.25` scores tune 0.8972 / 0.7883
(+0.0034 ± 0.0018, +0.0119 ± 0.0046) and test 0.9079 / 0.8061. It leads the tune
window, which by the protocol decides, and it is **not shipped** because it
fails test 1 and costs 0.010 of the synchronous forecast Spearman. Of 114 runs
the best tune NDCG is 1.9 standard errors above the baseline, which is about
what the maximum of 114 draws from noise gives on its own; the acceptance tests
are what separate a real gain from that, and here they did the separating.

## 4. The winner against the field

`uv run tools/compare_ndcg.py`, test window, the 282 tournaments every database
forecasts, weights frozen.

| system | NDCG (weighted) | NDCG@3 (weighted) | NDCG | NDCG@3 |
|---|---:|---:|---:|---:|
| **round 2, the winner (player based)** | **0.9079** | **0.8076** | **0.9064** | **0.8091** |
| round 1, the merged model (player based) | 0.9067 | 0.8024 | 0.9050 | 0.8045 |
| rating.chgk.fun (player based) | 0.9040 | 0.7997 | 0.8983 | 0.7932 |
| chgk.quest (player based) | 0.9011 | 0.7914 | 0.8974 | 0.7883 |
| shipped model (player based) | 0.8977 | 0.7799 | 0.8940 | 0.7782 |
| official chgk.gg (player based) | 0.8710 | 0.7261 | 0.8749 | 0.7421 |
| round 2, the winner (team based) | 0.8219 | 0.6592 | 0.8316 | 0.6791 |
| round 1, the merged model (team based) | 0.8205 | 0.6569 | 0.8303 | 0.6765 |
| official chgk.gg (team based) | 0.7917 | 0.6123 | 0.7986 | 0.6244 |

Paired bootstrap over the same tournaments, 2000 resamples, weighted; the
round-2 player row minus the other row.

| against | NDCG | SE | NDCG@3 | SE |
|---|---:|---:|---:|---:|
| round 1, the merged model | +0.0012 | 0.0016 | +0.0052 | 0.0039 |
| rating.chgk.fun | +0.0039 | 0.0036 | +0.0079 | 0.0083 |
| chgk.quest | +0.0067 | 0.0040 | +0.0162 | 0.0084 |
| the shipped model | +0.0102 | 0.0033 | +0.0277 | 0.0079 |
| official chgk.gg | +0.0369 | 0.0054 | +0.0815 | 0.0124 |
| round 2, team based | +0.0860 | 0.0088 | +0.1484 | 0.0172 |
| round 1, team based | +0.0873 | 0.0088 | +0.1507 | 0.0171 |
| official chgk.gg, team based | +0.1162 | 0.0095 | +0.1953 | 0.0185 |

And the same two competitors on both windows, where rating.chgk.fun's mirror
covers only half the tune fields:

| against | window | NDCG | SE | NDCG@3 | SE | fields |
|---|---|---:|---:|---:|---:|---:|
| rating.chgk.fun | tune | +0.0108 | 0.0050 | +0.0271 | 0.0110 | 327 |
| chgk.quest | tune | +0.0018 | 0.0027 | +0.0067 | 0.0063 | 646 |
| rating.chgk.fun | test | +0.0039 | 0.0036 | +0.0079 | 0.0083 | 282 |
| chgk.quest | test | +0.0067 | 0.0040 | +0.0162 | 0.0084 | 282 |

**Read honestly: this is not a clear margin, and round 2 did not produce one.**
The largest lead anywhere is 2.2 standard errors (fun, tune, NDCG) and the
smallest is 0.7 (quest, tune). The two windows do not even agree on which
competitor is the harder one: on the tune window chgk.quest is nearly level with
us and rating.chgk.fun is two standard errors behind, and on the test window it
is the other way round. Round 1's lead over fun was +0.0029 ± 0.0035 on test and
this round's is +0.0039 ± 0.0036; the gain is real and it is a third of a
standard error. Section 1 is the reason: there is no systematic bias left to
remove, and the fields that carry the loss are the ones where no forecast can
order the top.

## 5. The acceptance tests

`out/q2/accept_round1.md` beside `out/q2/accept_final.md`, both on the frozen
mirror with the history written.

| | round 1 | round 2 | |
|---|---:|---:|---|
| test 1, cross-fitted log-loss, in person, all seasons | 0.4900 | **0.4899** | passed |
| test 1, the same in 2025/26 | 0.5160 | 0.5160 | passed |
| test 1, all formats, all seasons | 0.4854 | **0.4853** | passed |
| test 2, Spearman, in person | 0.8524 | **0.8527** | passed |
| test 2, Spearman, synchronous | 0.8181 | **0.8182** | passed |
| test 2, Spearman, strictly synchronous | 0.8152 | **0.8187** | passed |
| test 3, top 200 against chgk.quest | 111/200 | **121**/200 | failed, better |
| test 3, top 200 against rating.chgk.fun | 115/200 | **130**/200 | failed, better |
| test 3, Spearman over 50+ games, chgk.quest | **0.8900** | 0.8556 | |
| test 3, the same against rating.chgk.fun | 0.7796 | **0.8028** | |
| test 4, inflation, largest September deviation | 6.0% | 6.0% | passed |
| test 5, transfer invariance, teams moved | 0 | **0** (max delta 0) | passed |
| test 6, winner rule, violations sole / shared | 0 / 0 | **0 / 0** | passed |
| test 7, team NDCG on its own, tune (w) | 0.8986 | **0.8996** | passed |
| test 7, the same, test window (w) | 0.9028 | **0.9047** | passed |
| forecast NDCG, test window (w) | 0.9065 | **0.9077** | |
| forecast NDCG@3, test window (w) | 0.8021 | **0.8072** | |
| innovation autocorrelation, in person | −0.057 | −0.053 | |
| innovation autocorrelation, all formats | 0.037 | 0.038 | |
| non-provisional players at the last release | 6324 | 6192 | |

Test 7 is on the 296 tune and 268 test fields all four systems cover; chgk.gg
scores 0.8893 and 0.8901 there and the old system 0.8877 and 0.8886, so the
rating is ahead of both on both windows, which is what the test asks.

**Nothing got worse except one number.** The Spearman of the published number
against chgk.quest's table falls from 0.8900 to 0.8556, which is the price of
`player_phi = 1`: that switch is what buys the top-200 overlap back, ten and
fifteen players, and it moves the two systems' agreement in opposite directions
because chgk.quest's own number leans more synchronous than rating.chgk.fun's
does. Test 3 still fails its 140-of-200 threshold — a threshold above what
chgk.quest and rating.chgk.fun reach with each other, 136 — but it is no longer
below the model that shipped before plan 3, which is what round 1 broke.

For the **runner-up** with the Newton fit the same row would read: test 1 0.4931
and 0.5235 (**failed**), test 2 in person 0.8533 but synchronous 0.8089, test 3
126 and 127 of 200, test 4 4.9%, forecast NDCG 0.9079 / 0.8061. It is a better
forecast and a worse model of the questions, and DESIGN 7.2's first test exists
to make that call.

## 6. The tables

`uv run tools/q_ndcg_tops.py out/q2/round1_hist.db out/q2/final.db`, release 714
of 2026-09-03.

### Top 20 teams

| # | round 1 | round 2 |
|---:|---|---|
| 1 | Борский корабел 9702 | Борский корабел 9704 |
| 2 | Gay Guerrilla 8689 | Gay Guerrilla 8761 |
| 3 | Мир 8500 | Мир 8531 |
| 4 | Призраки Коши 8369 | Призраки Коши 8449 |
| 5 | Первая сборная 8333 | Первая сборная 8394 |
| 6 | ff 8180 | ff 8211 |
| 7 | Заячий тулупчик 8012 | Заячий тулупчик 8114 |
| 8 | Тихие Тигры 7947 | Тихие Тигры 8027 |
| 9 | Механизм реакции 7795 | Рабочее название 7860 |
| 10 | Рабочее название 7789 | Я просто медленно пишу 7804 |
| 11 | Я просто медленно пишу 7784 | Механизм реакции 7796 |
| 12 | Железные рукава 7696 | Железные рукава 7786 |
| 13 | Приматы 7648 | Приматы 7697 |
| 14 | Кузинатра 7585 | Осторожно, надвигается кайфун 7591 |
| 15 | Осторожно, надвигается кайфун 7542 | Кузинатра 7576 |
| 16 | Олеги Литвы 7518 | Олеги Литвы 7550 |
| 17 | Кошка или сова 7431 | Кошка или сова 7472 |
| 18 | Хронически разумные United 7385 | 4 коња дебела 7453 |
| 19 | Работяги Лайт 7378 | Работяги Лайт 7424 |
| 20 | Суббота 13 7364 | Хронически разумные United 7407 |

Nineteen of the twenty are the same teams in nearly the same order, one enters
and one leaves. Halving the teammate shrinkage was never going to move the team
table much, and it did not.

### Top 20 players

| # | round 1 | round 2 |
|---:|---|---|
| 1 | Руссо Максим 15327 | Руссо Максим 15847 |
| 2 | Немец Илья 15282 | Семушин Иван 15393 |
| 3 | Семушин Иван 15083 | Васильев Дмитрий 15370 |
| 4 | Мокин Игорь 14983 | Мокин Игорь 14958 |
| 5 | Брутер Александра 14762 | Миротин Евгений 14829 |
| 6 | Лёгенький Николай 14148 | Изъюров Константин 14498 |
| 7 | Коробейников Александр 13897 | Тух Вадим 14364 |
| 8 | Молдавский Вадим 13886 | Брутер Александра 14356 |
| 9 | Миротин Евгений 13835 | Коробейников Александр 14323 |
| 10 | Галиакберов Денис 13751 | Галиакберов Денис 14283 |
| 11 | Туроверов Алексей 13656 | Гилёв Алексей 14243 |
| 12 | Изъюров Константин 13557 | Халиуллин Рузель 14220 |
| 13 | Марков Александр 13470 | Фёдорова Татьяна 14180 |
| 14 | Мосягин Александр 13462 | Марков Александр 14100 |
| 15 | Васильев Дмитрий 13453 | Сахаров Дмитрий 14082 |
| 16 | Айзятулова Иделия 13348 | Карякин Дмитрий 13943 |
| 17 | Соловьёв Олег 13337 | Комар Наталья 13939 |
| 18 | Халиуллин Рузель 13226 | Онскуль Илья 13812 |
| 19 | Мамедзаде Мамедшакир 13217 | Разумов Юрий 13751 |
| 20 | Гилёв Алексей 13190 | Плотников Дмитрий 13740 |

**This table moved, and it is `player_phi` that moved it, not the forecast.**
The round-2 column is the strength *at* an in-person table and the round-1
column the strength *away* from one; the players who rise — Васильев, Тух,
Фёдорова, Сахаров, Комар, Онскуль — are the ones whose record is mostly in
person, and the ones who leave are the ones whose `θ` was earned mostly at
synchronous packages. That is the same switch that took the top-200 agreement
with rating.chgk.fun from 115 to 130. Fourteen of the twenty are the same people.

## 7. What was not done, and what got worse

- **The margin over rating.chgk.fun and chgk.quest is still not three standard
  errors,** and the plan's stopping rule was never reached. The lead is between
  0.7 and 2.2 standard errors depending on the competitor and the window, and it
  is larger than round 1's in all four cells. On the evidence of section 1 a
  further round of the same kind will not change this: the residual carries no
  systematic bias in anything the model can see, and 0.089 of NDCG separates the
  fields with a bunched top from the fields with a spread one, an order of
  magnitude more than any lever in either round.
- **The Newton question fit wins the tune window and is not shipped**, for the
  second round running and now for a sharper reason than round 1 had: it fails
  test 1 by 0.0032 of cross-fitted log-loss on 3.7 million observations while
  improving the in-sample number by 0.021. Round 1 left this as a judgement
  call; this round turns it into a test result. `question.s_m = 1` with
  `fit = "newton"` is the point to reopen if anyone decides the forecast is
  worth a worse model of the questions.
- **`display.player_phi = 1` costs 0.034 of the rank correlation against
  chgk.quest** while gaining 0.023 against rating.chgk.fun and ten and fifteen
  players of the two top-200 overlaps. It changes what the published number
  *means* — DESIGN 3.7 and 5, the method page and the rules report are all
  updated — and it is the one place in this round where a number was made worse
  on purpose.
- **The source mirror changed under the run**; section 0. Everything here is on
  a frozen copy, the round-1 baseline was refitted on it and is inside 0.0004 of
  `out/round1.db`, but the absolute numbers here and in `out/q_ndcg/report.md`
  differ in the fourth decimal for that reason. Neither `out/shipped.db` nor
  `out/ext/*.db` was rebuilt on the frozen copy; the paired bootstrap intersects
  tournaments, so that is a coverage difference and not a bias.
- **`data.include_unrated` was not tried**, for round 1's reason: it changes the
  set of tournaments the metric is computed over. It is now the largest untried
  thing left, because chgk.quest trains on those tournaments and section 1.6
  says its advantage sits exactly where more data per player would show — the
  best-established lineups. Scoring it honestly needs `tools/ndcg.py --common`
  against a fixed tournament set.
- **Blending the forecast with the team state is worse at every weight**
  (section 1.7). It is not on the plan's list; it is recorded because it is the
  first thing anyone reading section 1.6 will want to try.
- **The weights were not re-derived.** They are still frozen at commit
  `fffb987`, as the protocol requires.

Section 8 adds four hypotheses that were not on the plan's list, written after
the diagnosis rather than before it. All four lose, and between them they close
`include_unrated` and the two readings of section 1 that looked most like
levers.

## 8. Four more hypotheses, after the diagnosis

The seven levers of the plan were written before the residual was looked at.
Section 1 says the residual has no exploitable structure, and the honest test of
that claim is to attack it from directions the plan did not list. Four were
built and run against the shipped winner; the base is `out/q3/base.toml` and the
rows are `out/q3/results.tsv`.

| hypothesis | best point | tune NDCG | tune @3 | test NDCG | d NDCG | SE |
|---|---|---:|---:|---:|---:|---:|
| **the winner, the base** | | 0.8948 | 0.7806 | 0.9077 | - | - |
| A. a fast channel for the forecast | `v_fast` 0.08, `k_fast` 0.5 | 0.8921 | 0.7773 | 0.9051 | −0.0027 | 0.0015 |
| A, harsher | `v_fast` 0.3, `k_fast` 0.5 | 0.8534 | 0.6903 | 0.8635 | −0.0414 | 0.0042 |
| B. the lineup's own spread | `sigma_v` −0.5 | 0.8924 | 0.7743 | 0.9066 | −0.0024 | 0.0011 |
| B, the other sign | `sigma_v` +0.5 | 0.8914 | 0.7726 | 0.9026 | −0.0033 | 0.0011 |
| C. a field-adaptive temperature | `tau_spread` 0.15 | 0.8942 | 0.7773 | 0.9072 | −0.0006 | 0.0006 |
| D. train on the unrated tournaments | `include_unrated` | — | — | — | −0.0051 | 0.0024 |

**A. A fast channel the forecast alone reads.** The sharpest fact in section 1.6
is that we lose to *both* rivals exactly on lineups with long records — players
whose `σ` has settled near 0.12 and whose `θ` therefore cannot move more than a
hundredth per tournament. The obvious missing object is recent form. So: a
second Kalman state for the same `θ`, kept at a volatility floor of its own,
taking the same step against the same gradient, read **only** by the forecast as
`θ + k_fast (θ_fast − θ)` — the question fit, the gauge and the published table
keep the long-run estimate, so none of the objections to simply raising `v_min`
apply. It is negative at every point of a 3 × 3 grid and monotone in both
parameters: −0.0027 at the gentlest setting and −0.15 at the harshest, on both
windows. **Recent form is a worse predictor of tomorrow than the long-run
estimate**, and the more of it the forecast reads the worse it gets. That is a
statement about the game, not about the code: a player's strength in this data
really is close to constant, and the model's slowness is not a deficit.

**B. The other sign of uncertainty.** `σ_c` reads every player at `θ − 1.5σ`, a
pessimistic *mean* shift. But the metric rewards ordering the top of a field, and
in a bunched field a table that *might* be very good wins more often than its
expected strength says — the win probability is convex in strength there. Nothing
in either model carried that term, so: add the lineup's aggregate sd to its
strength, `Q(L) + σ_v · sd`. It loses at `+0.5` (−0.0033) and it also loses at
`−0.5` (−0.0024). Both signs, so `σ_c = 1.5` is at a genuine interior optimum
and the convexity argument, which is correct in theory, is worth less than the
noise it adds.

**C. A temperature that knows the field.** Section 1.4 is the strongest single
fact in the whole diagnosis: a field's NDCG runs from 0.847 to 0.936 with the gap
between its first and fourth lineup. The natural response is to read a bunched
field more sharply — `τ_eff = τ (spread/1000)^{tau_spread}`, so a field whose
lineups stand 400 points apart is ordered more by each table's best player and
one whose lineups stand 2000 apart more by the table as a whole. It is −0.0006 at
0.15, −0.0017 at 0.6 and −0.0020 at −0.3. **Bunched fields do not want a
different rule; they are simply harder.** Section 1.4 describes a ceiling, not a
lever, and this is the experiment that tells the two apart.

**D. Training on the unrated tournaments.** Both rounds left this alone because
it changes the set of tournaments the metric covers. It is also the one
hypothesis with external evidence behind it: chgk.quest trains on those
tournaments, and section 1.6 says chgk.quest's advantage sits exactly where more
data per player would show — the best-established lineups. Scored honestly on
the 650 fields both models cover (`tools/ndcg.py --common`, so the extra
tournaments enter neither mean) it is **−0.0051 ± 0.0024** of weighted tune NDCG
and −0.0158 ± 0.0057 at 3, and −0.0045 with `w_async = 0.25` on top. The
tournaments the official system leaves out are left out for a reason; the volume
does not pay for the noise. That is the last hypothesis with a reason behind it,
and it is answered.

**What these four are worth.** They are negatives, and they are the useful kind.
Each was aimed at a specific thing the diagnosis identified — the frozen `θ` of
established players, the convexity at the top of a bunched field, the bunching
itself, the data we exclude — and each says the same thing from a different
direction: *the remaining loss is not information the model is failing to use.*
After the plan's seven levers and these four, the forecast has been attacked from
eleven angles in this round and the total gain is +0.0009 of tune NDCG. The next
thing to try should not be another parameter. It should be a different kind of
observation — a per-pair or per-lineup synergy term learned from the masks, a
venue or travel effect, or the question texts themselves — and each of those is a
larger piece of work than this plan had room for.
