# TrueDL, as it is and as it could be

Working notes for `truedl/`. Everything below is measured on `buff.db` (mirror of
rating.chgk.info) and on `truedl/ratings_snapshot.db`, a copy of the new rating system's
`out/ratings.db` taken on 2026-09-04 (5384 tournaments with fitted `question_params`,
714 weekly releases, 2013-01-04 to 2026-09-01).

Prototype: `truedl/prototype` (Rust, standalone workspace). Outputs: `truedl/out.csv`,
`truedl/report.txt`, `truedl/pairs.csv`. Upstream Python is mirrored in `truedl/upstream/`.

---

## 1. The current algorithm, as implemented

Two implementations exist upstream and they do not agree in the details:
`truedl/06_calculate_truedl.py` (batch, writes the Google-sheet table) and
`count_truedl_single_tourn.py` (the CLI users are pointed at). The published numbers on
rating.chgk.gg follow the CLI.

**Inputs.** For a tournament `t`: every team's result `Q_T` (count of `1` in the mask),
the number of questions `N`, and the *place* `P_T` of each team in the balanced ("Б")
rating release immediately preceding `date_start`.

**Coefficient table.** A step function of the rank band, derived in the blog post by
averaging, over tournaments where several bands met, the ratio of each band's mean score to
the top-10 band's mean score, then renormalising so that band 251-500 equals 1:

| place | 1-10 | 11-25 | 26-50 | 51-100 | 101-250 | 251-500 | 501-1000 | 1001-2000 | 2001-3000 | 3001-5000 |
|---|---|---|---|---|---|---|---|---|---|---|
| `C` | 1.61 | 1.52 | 1.43 | 1.32 | 1.16 | 1.00 | 0.81 | 0.60 | 0.43 | 0.31 |

**Per team.**

```
H_T  = min(Q_T / C(P_T), N)          hypothetical result of a 251-500 team
dl_T = (1 - H_T / N) * 10
```

**Per tournament.** `trueDL_t = mean over eligible teams of dl_T`. Eligible means
`rating.inRating` is true, the team appears in the release, and `P_T < 5000`. The batch
script additionally rescues teams missing from the release by finding a band whose observed
`rg` range brackets the team's `rg`, and truncates the hypothetical result with `int()`.
The CLI keeps `H_T` real-valued but rounds each `dl_T` to one decimal; the site publishes an
unrounded number, so the site does neither rounding nor truncation.

The old `DL` is kept for comparison: `DL_T = rg_T * (N - Q_T) / N / 500`, averaged over
teams with `rg > 3000`.

**What "DL" and "trueDL" mean.** Old `DL` is a rating-weighted miss rate: it rises both with
the difficulty of the package and with the rating scale of the era, which is why it inflates.
`trueDL` is meant to read as *"ten times the share of the package that a reference team --
one sitting at places 251-500 of the balanced rating -- would have missed."* `trueDL = 5.0` on
36 questions means such a team takes 18, and a top-10 team takes 29.

**How team strength enters.** Only through the rank band, and only as a multiplicative
constant. The model behind the formula is
`E[Q | band] = C(band) * E[Q | reference band]` -- a *proportional* model with no ceiling.

**Small fields.** No handling at all. The mean is taken over whatever eligible teams exist;
in this data 323 of 5300 scored tournaments are scored from fewer than five teams, and there
is no shrinkage, no weighting, and no uncertainty attached to the published number.

**Synchronous tournaments.** No handling. Same formula, same table, the release preceding
`date_start`. A live tournament and its synchronous mirror are two unrelated computations.

**Removed questions.** Handled inconsistently. The batch script sets
`N = len(mask_of_first_team) - count('X')`; the CLI sets `N = sum(questionQty)` and never
subtracts removals, so a tournament with a withdrawn question gets a different `N` depending
on which script is run. Neither script handles `?` (unknown) at all -- those positions count
as wrong in `questionsTotal` and stay in `N`.

### Reimplementation check

`truedl/prototype` reimplements the CLI formula in Rust and reproduces the published value.
Against 35 tournaments of 2024-2026 fetched from `rating.chgk.gg/api/v1/b/tournaments/<id>`
(`truedl/published_truedl.csv`):

| variant | pearson | spearman | rmse | median abs diff |
|---|---|---|---|---|
| unrounded | 0.9988 | 0.9989 | 0.098 | **0.008** |
| per-team rounded to 0.1 (CLI) | 0.9984 | 0.9980 | 0.111 | 0.037 |
| `int()` truncation (batch) | 0.9984 | 0.9964 | 0.124 | 0.106 |

So the site uses the unrounded form, and `truedl` in `out.csv` is that. The residual bias of
-0.038 is release-snapping and mask-vs-`questionsTotal` noise.

`buff.db` only carries balanced-release places from 2021-09-09 on (263 releases). For earlier
tournaments the prototype reconstructs a release: rank every team by the `rg` of its last
rated tournament, among teams with at least 6 rated games in the trailing 270 days. Tuned on
the overlap, that reconstruction agrees with the real release at spearman 0.856 and a median
place difference of 75; 60% of teams land in the same coefficient band. `out.csv` marks each
row `place_source = release | recon` (2580 / 2796 rows). **Every headline number below is
also reported for the release-backed era alone**, because the pre-2021 `truedl` column is a
proxy, not the published value.

---

## 2. Failure modes

### 2.1 The proportional model has no ceiling, so easy packages collapse to zero

`H_T = min(Q_T / C, N)` is clamped because `Q_T / C` routinely exceeds `N`. That happens for
2.3% of all eligible team-rows, in 19.6% of tournaments, and for more than a tenth of the
field in 6.3% of tournaments. Once a team is clamped its `dl_T` is exactly 0 and carries no
information; the measure has a hard floor and loses all resolution at the easy end.

The worst cases are tournaments played by a weak field:

| id | tournament | date | field | clamped | trueDL | improved |
|---|---|---|---|---|---|---|
| 13084 | Открытый чемпионат Эстонии среди школьников | 2026-01-03 | 24 | 96% | **0.01** | 1.00 |
| 13501 | Весенние зори 2026 | 2026-03-21 | 21 | 71% | 1.08 | 3.26 |
| 12830 | Интеллектуальный воронёнок — 2025 | 2025-11-22 | 39 | 62% | **0.76** | 4.43 |

Take 12830 concretely. 60 questions, 62 teams, mean score 23.2 (39%), median balanced place
2650, so `C = 0.43`. Scaling gives `23.2 / 0.43 = 54` and any team over 25 correct is clamped
at 60. The published number, 0.76, asserts that a team from places 251-500 would take 93% of
this package. The question model, fitted on the same masks, says 56%. The proportional model
is simply wrong here: a band ratio of 0.43 cannot survive contact with a probability that has
to stay below 1.

### 2.2 The number depends on who showed up

If the band ratios were exact, the mean over the field would be composition-free. They are
not, and the residual leaks. Regressing `truedl` on the improved measure across 5300
tournaments and correlating the residual with field composition:

- corr(residual, log median balanced place of the field) = **-0.256**
- corr(residual, share of the field in the top 100) = **+0.175**
- corr(residual, share of clamped teams) = **-0.335**

A package played by weaker teams is scored easier than the same package played by stronger
teams, by construction.

### 2.3 The same package, live and mirrored, gets two different numbers

28 pairs were identified in 2013-2026 as "one live tournament and one synchronous run of the
same package": identical editor list, identical tour structure, dates within 60 days, and
name-token Jaccard >= 0.5 (`truedl/pairs.csv`). The package is the same; the difficulty
should be the same.

| | median abs diff | mean abs diff | correlation across pairs |
|---|---|---|---|
| trueDL | 0.561 | 0.657 | 0.864 |
| improved | **0.362** | **0.395** | **0.933** |

And the difference is not just noise, it has a sign. Mirror minus live:

- trueDL: mean **+0.357**, median **+0.364**
- improved: mean +0.134, median +0.128

The synchronous run draws a broader, weaker field, so trueDL systematically calls it a third
of a point harder than the live tournament it copies. Examples straight from the list:
Мюнхен зовёт 2023 3.9 live / 5.4 mirror, Мюнхен зовёт! 2025 3.8 / 5.1, Мюнхен зовёт 2024
3.7 / 4.9, Zhodino Minor 2.9 / 4.7. In every one of those the improved measure moves by
0.1-0.7 instead of 1.2-1.5.

Restricted to the release-backed era (17 pairs, real published places): trueDL median
|diff| 0.758, improved 0.314.

### 2.4 Small and truncated fields

TrueDL scores a tournament from its *eligible* teams only, on average 61 of the 95 teams the
question model sees. In 20% of tournaments it uses fewer than half the field; 76 tournaments
cannot be scored at all because no participant appears in the release. School and mass
synchronous tournaments are hit hardest -- precisely the ones where the surviving teams are
the strongest present, which is where the clamp bites.

With no shrinkage, small fields give wild numbers. The least stable tournaments under
half-field subsampling (section 4.2):

| id | tournament | eligible | trueDL | rms | improved | rms |
|---|---|---|---|---|---|---|
| 10365 | Весеннее обострение | 11 | 3.22 | **1.80** | 2.84 | 0.41 |
| 11742 | ближе к близко | 10 | 2.70 | 1.50 | 2.32 | 0.11 |
| 12254 | Бужский Гард-8 | 10 | 4.28 | 1.11 | 5.06 | 0.20 |
| 6223 | С чаем пойдёт (green) | 9 | 5.04 | 1.08 | 3.95 | 0.24 |
| 10663 | Синхронный Кубок Ярослава Мудрого 2024, ч. 2 | 9 | 2.77 | 1.05 | 1.49 | 0.25 |

Dropping a random half of an eleven-team field moves the published difficulty of
Весеннее обострение by nearly two points on a ten-point scale.

### 2.5 Discontinuity, rank drift, circularity, data hygiene

- **Steps.** The band table is a step function: place 10 gives 1.61, place 11 gives 1.52. A
  single place moves a team's `dl_T` by roughly 0.3 on packages of ordinary difficulty.
- **Rank drift.** TrueDL is inflation-proof only if "place 500" means the same thing over
  time. The balanced release listed 4396 teams at its first snapshot and 3195 at the last;
  the pool of teams with a rated game in the trailing year fell from ~4800 to ~3000 after
  2022. Place 500 in 2016 and place 500 in 2026 are not the same strength.
- **Circularity.** The place comes from the balanced rating, which is itself computed from
  results on these packages; the anchor moves with the thing being measured.
- **Broken masks pass straight through.** Чемпионат Санкт-Петербурга. Первая лига (5283,
  6149) and Чемпионат Минска (2577) have all-zero masks in the mirror and score `trueDL =
  10.0`. Neither measure guards against this; a filter belongs in whatever consumes them.

---

## 3. The improved measure

### 3.1 Definition

The new rating system already fits, per question `q` of tournament `t`, a difficulty `b_q` on
the player-strength scale, a log-discrimination `log a_q`, and a step count `m_q`
(DESIGN section 3.2; the current config fixes `m_q = 1`, `s_m = 0`). For a lineup `L`:

```
lambda_iq     = exp( a_q * (theta_i - b_q) )
P(L solves q) = ( 1 - exp( -sum_{i in L} lambda_iq ) ) ^ m_q
```

Fix a **reference lineup**: six identical players of strength `theta*`. Then

```
p_q(theta*)   = ( 1 - exp( -6 * exp( a_q * (theta* - b_q) ) ) ) ^ m_q

E_t(theta*)   = ( 1 / |Q_t| ) * sum_{q in Q_t} p_q(theta*)     Q_t = questions with an answer

TrueDL*_t     = 10 * ( 1 - E_t(theta*) )
```

with the anchor

```
theta* = median over lineups of teams at balanced places 251-500 of
         log( ( sum_{i in L} e^{theta_i} ) / 6 )   =  0.2368
```

measured over 20 993 such lineups in 2024-2026. That is the roster strength (DESIGN section
4.1) of the band TrueDL already calls its reference, re-expressed as six equal players.
**The stated meaning of the number is exactly the one TrueDL claims** -- ten times the share
of the package a 251-500 team would miss -- but it is computed from the fitted question
difficulties instead of from a band-ratio table.

A scale-free companion, also in `out.csv`:

```
theta_half_t = the theta solving E_t(theta) = 0.5
```

the strength at which a six-player lineup takes half the package. This one needs no anchor
at all and is directly comparable to player ratings.

### 3.2 Why it fixes what it fixes

- **Anchored to the population, not to the field.** `theta = 0` is the mean of established
  players at each release by the gauge rule (DESIGN section 3.5), so `theta*` does not drift;
  inflation is impossible for the same reason it is impossible for `theta`.
- **No ceiling problem.** `p_q` lies strictly in (0,1), so there is no clamp, no floor at
  zero, and the easy end keeps its resolution (12830: 4.43 rather than 0.76).
- **Uses the whole field.** `b_q` is fitted from every team that answered, not from the ~64%
  that happen to be in the release.
- **Degrades gracefully.** `b_q ~ N(1.5, 2^2)` means a question with few observations shrinks
  toward the middle instead of producing a wild estimate, so tiny fields produce conservative
  numbers rather than noisy ones.
- **Removed questions drop out for free.** A question with no observations has no
  contribution; `X` and `?` never enter `Q_t`. No two implementations can disagree about `N`.
- **Composable.** It is defined per question, so a tour, an arbitrary question range, or a
  planned synchronous subset of a live package is scored by averaging `p_q` over that subset --
  which is the use case the CLI's `-q` flag exists for, and it becomes exact instead of a
  re-scaling of partial team scores.

### 3.3 Cost

It requires the question model to have been fitted, so it is downstream of the rating run and
cannot be computed for a tournament outside the system (the current CLI can score a package
from a spreadsheet alone). For organisers wanting a pre-release estimate, the honest answer is
a forecast object (DESIGN principle 6), not this measure.

---

## 4. Evaluation

All numbers from `truedl/report.txt`, 5376 tournaments since 2013.

### 4.0 The refit reproduces the online fit

The stability test needs the improved measure recomputed on half a field, which means
refitting `b_q`. The prototype does its own Newton fit of `(b_q, log a_q)` on the penalised
log-likelihood with the config priors, taking player `theta` from the release preceding the
tournament. Refitting on the **full** field and comparing against the parameters the online
fit stored:

- pearson 0.9983, spearman 0.9983, rmse 0.087 on the 0-10 scale.

So the refit is a faithful stand-in and the subsampling comparison is measuring the estimator,
not my reimplementation.

### 4.1 Agreement with the published TrueDL

`truedl = 0.074 + 0.944 * improved`, pearson 0.906, spearman 0.907 over 5300 tournaments.
Means and spreads are nearly identical -- trueDL 4.94 +/- 1.51, improved 5.16 +/- 1.45 --
which is the anchor doing its job, not a fitted rescaling.

| subset | n | spearman | pearson |
|---|---|---|---|
| all, 2013-2026 | 5300 | 0.907 | 0.906 |
| release-backed places only (2021-09 on) | 2570 | **0.946** | 0.934 |
| 2024-2026 | 1540 | **0.951** | 0.939 |
| 2013-2019 (reconstructed places) | 2597 | 0.868 | 0.879 |

The gap between the eras is mostly my place reconstruction, not a real change in agreement.
Read as: *the ranking of tournaments barely moves, individual numbers move by a few tenths,
and they move most where TrueDL was breaking down.*

Both measures agree at the hard end and both reproduce the blog's list: Часы Судного Дня:
Полночь 9.10 / 9.40 (blog: 9.2), Посох Аарона 8.82 / 9.23 and 8.83 / 9.18 (blog: 9.0),
Вавилонский колодец 8.52 / 9.02 (blog: 8.5).

### 4.2 Stability under subsampling the field

Six random halves of each field; each half is scored by both measures -- TrueDL by averaging
its eligible teams in the half, the improved measure by refitting every `b_q` on the half --
and the deviation is the rms against the full-field value. 5082 tournaments with at least 8
teams and 12 questions.

| | median rms | mean rms | spread across tournaments | noise / signal |
|---|---|---|---|---|
| trueDL | 0.168 | 0.220 | 1.48 | 0.114 |
| improved | **0.115** | **0.142** | 1.44 | **0.085** |

By field size (median rms, improved rescaled to the trueDL scale):

| field | n | trueDL | improved |
|---|---|---|---|
| 8-20 teams | 843 | 0.340 | **0.253** |
| 21-60 | 1985 | 0.216 | **0.146** |
| 61-200 | 1644 | 0.126 | **0.083** |
| 201+ | 610 | 0.078 | **0.050** |

The improved measure is more stable at every field size, by a factor of 1.3-1.5, while having
essentially the same spread across tournaments -- so the gain is not bought with compression.

### 4.3 Same package, two runs

Section 2.3: median |diff| 0.362 against 0.561, pair correlation 0.933 against 0.864, and the
systematic mirror-is-harder bias shrinks from +0.36 to +0.13. On the 17 release-backed pairs,
0.314 against 0.758.

Caveat: 28 pairs is small, and the matching heuristic can still pair a package with a
different package by the same editors in the same weeks. Errors of that kind hurt both
measures equally, so the comparison stands, but the absolute levels should not be
over-read. The pair list is in `truedl/pairs.csv` and should be eyeballed before quoting.

---

## 5. Recommendation

Adopt `TrueDL* = 10 * (1 - E_t(theta*))` with the six-player reference lineup at
`theta* = 0.2368`, keep the familiar 0-10 presentation, and publish `theta_half` next to it
for anyone who wants the scale-free version.

The case: it is at least as stable as TrueDL everywhere it was tested (1.3-1.5x better under
field subsampling, 1.5-2.4x better across live/mirror pairs), it removes the systematic
mirror-is-harder bias, it recovers the easy end that the clamp destroys, it uses the whole
field instead of the ~64% in the rating, and it carries the same sentence of meaning that
made TrueDL worth having. Rank correlation with the published numbers is 0.95 in the current
era, so the table of hardest and easiest tournaments will look familiar; the tournaments whose
number moves are the school and mass-synchronous ones, where TrueDL was reporting 0.

Three things to settle before shipping:

1. **The anchor is a choice.** `theta* = 0.2368` reproduces TrueDL's scale, but it is pinned
   to the 251-500 band of a rating whose population is shrinking. Recomputing it each year
   would reintroduce exactly the rank drift TrueDL was built to avoid. Fix it once, publish
   it, and let the number drift with the field's real strength.
2. **Publish an uncertainty.** The question fit gives a posterior on `b_q`; propagating it to
   `E_t` is cheap and would let the site show +/-0.2 on a twelve-team tournament and +/-0.05
   on a three-hundred-team one instead of pretending both are exact.
3. **Add a sanity filter.** All-zero masks currently score 10.0 under either measure.

---

## 6. Running the prototype

```
cd truedl/prototype
cargo run --release            # writes ../out.csv, ../report.txt, ../pairs.csv
cargo run --release -- tune    # place-reconstruction grid search
```

It reads `buff.db` and `truedl/ratings_snapshot.db` read-only and touches nothing in the
root workspace. Runtime about 40 s on four cores; the seven refits per tournament dominate.

`out.csv` columns: `tournament_id, date, type, n_teams, n_teams_truedl, n_q, place_source,
truedl, improved, truedl_sub_sd, improved_sub_sd, truedl_sub_rms, improved_sub_rms,
theta_half, improved_refit, median_place, share_top100, share_clamped`.
`n_teams` is the field the question model sees; `n_teams_truedl` is the subset TrueDL can
score. `improved` uses the stored `question_params`; `improved_refit` uses the prototype's own
full-field refit, and the `_sub_*` columns are measured against it.
