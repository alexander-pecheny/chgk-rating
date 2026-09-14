# Plan 6: the question model, third round — new kinds of signal

Round 2 ended with a diagnosis rather than a gain: over the weighted in-person
fields of the tune window, **no property of a lineup the model can see predicts
the residual above 0.10** once the lineup's own forecast rank is controlled for,
and 90% of what is left sits on fully measured lineups in fields whose top four
are bunched. Plan 6 asked for signals that are not properties of a single
player. Seven were built. Six of them lost, most of them badly, and each loss is
recorded with the mechanism that predicted it would win.

The seventh is the one thing in three rounds that moves both windows and both
metrics in the same direction. It is not any of the six the plan listed: it came
out of the residual check the plan asked for, run on a lever the user added
mid-round, and it says the opposite of what that lever assumed. **The editor
effect is real, it is a property of a strong lineup, and it does not live in the
question gradients — it lives in the outcomes.**

What ships is one switch and one constant:

```
[team.s2] editor_memory = 1250.0   ed_gate = 0.2   ed_min_n = 2.0
```

```
ndcg tune 0.8963 @3 0.7834  unweighted 0.9014 @3 0.7988  (650 tournaments)
ndcg test 0.9084 @3 0.8108  unweighted 0.9070 @3 0.8122  (284 tournaments)
paired tune minus round 2: +0.0015 se 0.0010  @3 +0.0028 se 0.0031
paired test minus round 2: +0.0007 se 0.0008  @3 +0.0035 se 0.0023
```

Every acceptance number is bit-identical to round 2 except the one the change is
meant to move. **The lead over rating.chgk.fun and chgk.quest is still not three
standard errors**, and this round did not make it so either; it is now 1.3 and
1.9 on the test window and 2.4 and 1.2 on the tune window.

## 0. The baseline

Everything is on the frozen mirror
`/mnt/HC_Volume_106816389/build/herdr-worktrees/r2out/buff_frozen.db`, symlinked
as `buff.db`. The round-2 winner, refitted from `config.toml` at `5b23144` with
`--to 2026-09-05 --rows-every 0`:

```
ndcg tune 0.8948 @3 0.7806  unweighted 0.9003 @3 0.7972  (650 tournaments)
ndcg test 0.9077 @3 0.8072  unweighted 0.9063 @3 0.8089  (284 tournaments)
```

Identical to `out/q_ndcg/round2.md` section 3 in every digit, and `6192` of
`25061` players non-provisional, which is round 2's acceptance row. The
eight-row comparison reproduces round 2's 0.9079 / 0.8076 on the 282 common test
tournaments exactly. Protocol unchanged: tune 2018-09-01 to 2024-08-29 decides,
test 2024-08-30 to 2026-08-27 is read once per row, weights frozen at `fffb987`,
paired bootstrap of 2000 resamples. **93 fits**, `out/q_ndcg/results3.tsv`,
driven from `out/q_ndcg/batch3/`.

## 1. The mechanisms, checked before the sweeps

`uv run tools/q_mech.py out/q3/base.db --lever N`, on the object plan 5 used:
one row per in-person result in a weighted tune field — 17 392 results in 581
fields — the residual `(forecast rank − actual rank) / n` with the mean residual
of the team's forecast-percentile bin subtracted. The whole table is
`out/q_ndcg/round3_mech.md`.

### 1.1 Chemistry (lever 1)

`T(L)`, the mean over a lineup's pairs of `log(1 + rated games together)`,
counted over everything before the field's own date.

| feature | weighted corr with the residual | results |
|---|---:|---:|
| `T(L)` | +0.015 | 17 371 |
| the same, largest pair | +0.004 | 17 371 |

The plan asked specifically whether the winning 5th-to-10th forecasts are stable
lineups and the losing favourites are not. Among the ten highest forecasts of
fields of 20 teams or more:

| cohort | results | mean `T(L)` |
|---|---:|---:|
| forecast 1-3, finished 1-3 | 783 | 2.90 |
| forecast 1-3, finished 4+ | 447 | **2.99** |
| forecast 4-10, finished 1 | 71 | **2.96** |
| forecast 4-10, finished 2-3 | 284 | 2.76 |
| forecast 4-10, finished 4+ | 2502 | 2.69 |

**Half of the premise holds and half of it is backwards.** The surprise
winners — forecast 4th to 10th and first past the post — are the settled
lineups, 2.96 against 2.69 for the ones that stayed where they were put. But the
favourites who disappointed are *more* settled than the ones who delivered, 2.99
against 2.90. Togetherness therefore marks a team that can beat its forecast and
equally a team that can fall short of it; it has no sign, which is why a term
linear in `T(L)` cannot use it. The sweep confirms this at both signs.

### 1.2 Difficulty against lineup shape (lever 2)

`m ≠ 1` is worth having only if how hard the package is changes what a
star-plus-passengers lineup is worth against a balanced one. The spread of `θ`
inside the lineup against the residual, by tercile of the field's difficulty:

| tercile of the field's difficulty | truedl | results | corr(spread of `θ`, residual) |
|---|---:|---:|---:|
| 1 | 4.13 | 5770 | +0.020 |
| 2 | 5.54 | 5770 | +0.001 |
| 3 | 6.54 | 5770 | +0.014 |

Not monotone, not ordered, every cell under 0.021, and the overall correlation is
−0.003. **There is no interaction to fit**, and the sweep found none.

### 1.3 The record the rated window cannot see (lever 3)

Unrated tournaments each lineup's players played in the two years before the
field. 88.7% of lineups have at least one such player and the mean count is 7.07,
so the data is there in quantity; it simply does not predict.

| feature | weighted corr with the residual | results |
|---|---:|---:|
| median unrated tournaments per player, last 2 years | +0.011 | 17 390 |
| the mean of the same | +0.011 | 17 390 |

### 1.4 The lineup's own uncertainty (lever 4)

| feature | weighted corr with the residual | results |
|---|---:|---:|
| mean `σ` of the lineup | +0.006 | 17 390 |
| `sd(Q(L))`, propagated through the tempered roster estimate | +0.001 | 17 390 |

`sd(Q(L))` is 0.39 of the mean `σ` on average and **correlates with it at
0.932**: the two are the same number up to a scale, so the lineup-level quantile
is the per-player quantile with `σ_c` rescaled, and round 2's `sigma_v` already
answered it. Run anyway, small, for completeness.

### 1.5 How much a point of forecast gap buys (lever 5)

| decile of the 1st-to-2nd forecast gap | gap (points) | fields | P(the favourite wins) |
|---|---:|---:|---:|
| 1 | 28 | 53 | 0.275 |
| 2 | 82 | 53 | 0.333 |
| 3 | 147 | 54 | 0.315 |
| 4 | 207 | 53 | 0.447 |
| 5 | 273 | 54 | 0.275 |
| 6 | 375 | 53 | 0.569 |
| 7 | 510 | 53 | 0.613 |
| 8 | 680 | 54 | 0.647 |
| 9 | 894 | 53 | 0.691 |
| 10 | 1468 | 54 | 0.872 |

Monotone from the fourth decile on and well behaved throughout: a gap of 1468
points buys 0.87 and a gap of 28 buys 0.28, which is what a calibrated
discrimination looks like. **Nothing here asks for `a ≠ 1`**, and the sweep
agrees at 0.8, 1.2 and 1.5.

### 1.6 Series memory (lever 6)

A team's mean surprise over its last five in-person results against its residual
today — the autocorrelation of the residual per team.

| feature | weighted corr with the residual | results |
|---|---:|---:|
| mean of the last 5 surprises | +0.032 | 13 069 |
| teams with 5+ in-person results behind them | +0.016 | 10 655 |
| teams with 10+ | +0.015 | 8365 |
| teams with 20+ | **+0.004** | 5755 |
| the five highest forecasts of a field | +0.033 | 2559 |

The correlation **falls towards zero as the team's record lengthens**, from
+0.032 at two results to +0.004 at twenty. That is the signature of an estimate
that has not settled yet, not of a persistent property of the team: a team with
two in-person results has a noisy forecast and its errors repeat because the
forecast is wrong in the same direction twice. By twenty results there is nothing
left. The sweep found what that predicts — nothing at either sign that holds.

### 1.7 Editors and strength (the user's lever)

The lineup's mean past residual under today's editors, against its residual here.
The accumulator only ever sees results strictly before the field, so this is
causally clean.

| cohort | weighted corr(mean past residual under these editors, residual) | results |
|---|---:|---:|
| every lineup with any history under the editors | +0.013 | 8834 |
| the weakest two fifths by mean `θ` | +0.003 | 3532 |
| the middle two fifths | +0.000 | 3532 |
| the strongest fifth | **+0.055** | 1770 |
| the strongest twentieth | **+0.098** | 441 |

50.8% of the results have any history under the tournament's editors at all.

**This is the only feature in three rounds of diagnosis to reach 0.10 in any
cohort**, and it is exactly the user's hypothesis: a weak player misses a
question whoever wrote it, a strong one has editors they read well. The gradient
across the strength quintiles is clean — 0.003, 0.000, 0.055, 0.098 — and it is
not a floor effect, because the forecast-percentile control is already in.

## 2. The levers

`d` is the paired difference against the round-2 baseline with its bootstrap
standard error, tune window, 2000 resamples. Full table `out/q_ndcg/results3.tsv`.

| lever | tune NDCG | tune @3 | test NDCG | test @3 | d NDCG | SE | d @3 | SE |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **round 2, the baseline** | 0.8948 | 0.7806 | 0.9077 | 0.8072 | - | - | - | - |
| **1. chemistry** | | | | | | | | |
| `chemistry` 50 | 0.8945 | 0.7772 | 0.9029 | 0.7974 | −0.0003 | 0.0012 | −0.0034 | 0.0029 |
| `chemistry` 100 | 0.8919 | 0.7712 | 0.9015 | 0.7927 | −0.0028 | 0.0019 | −0.0094 | 0.0043 |
| `chemistry` 200 | 0.8834 | 0.7504 | 0.8918 | 0.7712 | −0.0114 | 0.0027 | −0.0302 | 0.0060 |
| `chemistry` 400 | 0.8545 | 0.6913 | 0.8630 | 0.7042 | −0.0402 | 0.0037 | −0.0893 | 0.0086 |
| `chemistry` −100 | 0.8936 | 0.7781 | 0.9032 | 0.7936 | −0.0012 | 0.0015 | −0.0024 | 0.0040 |
| `chem_u` 0.05 | 0.8929 | 0.7766 | 0.9030 | 0.7940 | −0.0018 | 0.0015 | −0.0040 | 0.0041 |
| `chem_u` 0.1 | 0.8869 | 0.7673 | 0.8931 | 0.7734 | −0.0078 | 0.0024 | −0.0133 | 0.0056 |
| `chem_u` 0.2 | 0.8633 | 0.7127 | 0.8695 | 0.7256 | −0.0314 | 0.0035 | −0.0679 | 0.0081 |
| `chem_u` −0.1 | 0.8826 | 0.7550 | 0.8944 | 0.7713 | −0.0121 | 0.0023 | −0.0255 | 0.0056 |
| `chem_u` 0.1 + `chemistry` 100 | 0.8893 | 0.7695 | 0.9003 | 0.7920 | −0.0054 | 0.0020 | −0.0110 | 0.0048 |
| **2. one global `m`, and the difficulty-aware forecast** | | | | | | | | |
| `m_fixed` 0.9 | 0.8944 | 0.7802 | 0.9075 | 0.8055 | −0.0004 | 0.0006 | −0.0004 | 0.0017 |
| `m_fixed` 1.05 | 0.8951 | 0.7820 | 0.9079 | 0.8069 | +0.0003 | 0.0003 | +0.0014 | 0.0014 |
| **`m_fixed` 1.1** | **0.8951** | **0.7825** | 0.9077 | 0.8061 | **+0.0004** | 0.0005 | **+0.0019** | 0.0015 |
| `m_fixed` 1.15 | 0.8944 | 0.7812 | 0.9062 | 0.8032 | −0.0004 | 0.0007 | +0.0006 | 0.0017 |
| `m_fixed` 1.25 | 0.8937 | 0.7790 | 0.9061 | 0.8051 | −0.0010 | 0.0008 | −0.0016 | 0.0018 |
| `m_fixed` 1.5 | 0.8930 | 0.7775 | 0.9029 | 0.7970 | −0.0017 | 0.0013 | −0.0031 | 0.0026 |
| `m_fixed` 2.0 | 0.8919 | 0.7752 | 0.9018 | 0.7951 | −0.0029 | 0.0016 | −0.0053 | 0.0039 |
| `difficulty_forecast`, `m` 1.0 | 0.8925 | 0.7731 | 0.9033 | 0.7982 | −0.0023 | 0.0013 | −0.0074 | 0.0033 |
| `difficulty_forecast`, `m` 1.1 | 0.8925 | 0.7728 | 0.9022 | 0.7940 | −0.0023 | 0.0014 | −0.0078 | 0.0034 |
| `difficulty_forecast`, `m` 1.25 | 0.8905 | 0.7714 | 0.8997 | 0.7875 | −0.0042 | 0.0015 | −0.0092 | 0.0035 |
| `difficulty_forecast`, `m` 1.5 | 0.8883 | 0.7662 | 0.8997 | 0.7884 | −0.0065 | 0.0017 | −0.0144 | 0.0038 |
| `difficulty_forecast`, `m` 2.0 | 0.8861 | 0.7602 | 0.8976 | 0.7848 | −0.0087 | 0.0020 | −0.0204 | 0.0045 |
| **3. train on the unrated tournaments** | | | | | | | | |
| `train_unrated`, `w_unrated` 0.25 | 0.8950 | 0.7759 | 0.9054 | 0.8042 | +0.0002 | 0.0015 | −0.0046 | 0.0038 |
| `train_unrated`, `w_unrated` 0.5 | 0.8949 | 0.7763 | 0.9040 | 0.8049 | +0.0002 | 0.0018 | −0.0043 | 0.0045 |
| `train_unrated`, `w_unrated` 1.0 | 0.8896 | 0.7648 | 0.9036 | 0.7980 | −0.0051 | 0.0024 | −0.0158 | 0.0056 |
| **4. a lineup-level quantile** | | | | | | | | |
| `q_sigma_c` 0.5 | 0.8950 | 0.7807 | 0.9069 | 0.8055 | +0.0002 | 0.0003 | +0.0001 | 0.0011 |
| `q_sigma_c` 1.0, `sigma_c` 0.75 | 0.8951 | 0.7797 | 0.9067 | 0.8058 | +0.0004 | 0.0005 | −0.0009 | 0.0013 |
| `q_sigma_c` 1.5, `sigma_c` 0 | 0.8952 | 0.7788 | 0.9068 | 0.8050 | +0.0004 | 0.0008 | −0.0018 | 0.0017 |
| **5. discrimination by format** | | | | | | | | |
| `a_in_person` 0.8 | 0.8933 | 0.7764 | 0.9050 | 0.8004 | −0.0015 | 0.0012 | −0.0041 | 0.0034 |
| `a_in_person` 1.2 | 0.8926 | 0.7733 | 0.9066 | 0.8014 | −0.0022 | 0.0012 | −0.0073 | 0.0028 |
| `a_in_person` 1.5 | 0.8886 | 0.7643 | 0.9025 | 0.7957 | −0.0061 | 0.0018 | −0.0163 | 0.0040 |
| **6. series memory** | | | | | | | | |
| `kappa_s` 200 | 0.8944 | 0.7790 | 0.9068 | 0.8060 | −0.0003 | 0.0002 | −0.0015 | 0.0012 |
| `kappa_s` 500 | 0.8944 | 0.7790 | 0.9070 | 0.8051 | −0.0004 | 0.0005 | −0.0016 | 0.0020 |
| `kappa_s` 1000 | 0.8942 | 0.7800 | 0.9060 | 0.8011 | −0.0005 | 0.0010 | −0.0006 | 0.0026 |
| `kappa_s` 2000 | 0.8927 | 0.7759 | 0.9038 | 0.7983 | −0.0021 | 0.0013 | −0.0047 | 0.0035 |
| `kappa_s` −250 | 0.8950 | 0.7819 | 0.9066 | 0.8045 | +0.0002 | 0.0005 | +0.0009 | 0.0016 |
| **`kappa_s` −500** | **0.8961** | **0.7833** | 0.9067 | 0.8046 | **+0.0013** | 0.0007 | **+0.0027** | 0.0023 |
| `kappa_s` −1000 | 0.8949 | 0.7798 | 0.9066 | 0.8030 | +0.0002 | 0.0011 | −0.0006 | 0.0026 |
| `kappa_s` −2000 | 0.8916 | 0.7758 | 0.8998 | 0.7903 | −0.0032 | 0.0018 | −0.0084 | 0.0042 |
| `kappa_s` −500, `series_k` 3 / 10 | 0.8946 / 0.8951 | 0.7797 / 0.7812 | 0.9064 / 0.9068 | 0.8032 / 0.8038 | −0.0002 / +0.0004 | 0.0008 | −0.0002 / −0.0011 | 0.0020 |
| **7a. editor affinity in the question gradients** | | | | | | | | |
| `aff_rank` 1, flat (round 2's row) | 0.8921 | 0.7753 | 0.9054 | 0.8013 | −0.0027 | 0.0015 | −0.0053 | 0.0039 |
| `scale`, `aff_kappa` 0.3 | 0.8920 | 0.7745 | 0.9032 | 0.7964 | −0.0028 | 0.0016 | −0.0060 | 0.0040 |
| `scale`, `aff_kappa` 0.5 | 0.8924 | 0.7757 | 0.9042 | 0.7999 | −0.0023 | 0.0015 | −0.0049 | 0.0040 |
| `scale`, `aff_kappa` 1.0 | 0.8884 | 0.7662 | 0.8997 | 0.7890 | −0.0064 | 0.0021 | −0.0143 | 0.0045 |
| `scale`, `aff_kappa` 2.0 | 0.8635 | 0.7120 | 0.8944 | 0.7753 | −0.0313 | 0.0035 | −0.0686 | 0.0080 |
| `gate` 0.5 | 0.8900 | 0.7713 | 0.9027 | 0.7946 | −0.0048 | 0.0017 | −0.0093 | 0.0041 |
| `gate` 0.2 | 0.8908 | 0.7707 | 0.9043 | 0.8003 | −0.0040 | 0.0017 | −0.0099 | 0.0042 |
| `gate` 0.05 | 0.8924 | 0.7763 | 0.9050 | 0.8012 | −0.0024 | 0.0013 | −0.0043 | 0.0031 |
| `gate` 0.02 | 0.8946 | 0.7803 | 0.9054 | 0.8046 | −0.0001 | 0.0009 | −0.0002 | 0.0023 |
| `gate` 0.01 | 0.8952 | 0.7803 | 0.9070 | 0.8050 | +0.0004 | 0.0004 | −0.0003 | 0.0011 |
| `gate` 0.05, `s_aff` 0.6 | 0.8905 | 0.7729 | 0.9018 | 0.7916 | −0.0043 | 0.0015 | −0.0077 | 0.0033 |
| `gate` 0.05, `s_aff` 0.15 | 0.8950 | 0.7808 | 0.9077 | 0.8079 | +0.0003 | 0.0004 | +0.0002 | 0.0006 |
| `gate` 0.05, `eta_v` 0.5 | 0.8923 | 0.7751 | 0.9037 | 0.8000 | −0.0025 | 0.0012 | −0.0054 | 0.0028 |
| `gate` 0.2, `aff_rank` 2 | 0.8911 | 0.7731 | 0.9036 | 0.7961 | −0.0037 | 0.0019 | −0.0075 | 0.0045 |
| `scale` 1.0, `aff_rank` 2 | 0.8760 | 0.7403 | 0.8940 | 0.7743 | −0.0188 | 0.0028 | −0.0402 | 0.0069 |
| **7b. the editor memory, in the outcomes** | | | | | | | | |
| `editor_memory` 500 | 0.8947 | 0.7818 | 0.9076 | 0.8083 | −0.0000 | 0.0003 | +0.0012 | 0.0021 |
| `editor_memory` 750 | 0.8953 | 0.7827 | 0.9081 | 0.8093 | +0.0006 | 0.0006 | +0.0022 | 0.0023 |
| `editor_memory` 1000 | 0.8956 | 0.7837 | 0.9084 | 0.8110 | +0.0008 | 0.0007 | +0.0031 | 0.0026 |
| `editor_memory` 1250 | 0.8958 | 0.7836 | 0.9079 | 0.8099 | +0.0010 | 0.0010 | +0.0030 | 0.0031 |
| `editor_memory` 1500 | 0.8960 | 0.7839 | 0.9072 | 0.8080 | +0.0013 | 0.0010 | +0.0034 | 0.0031 |
| `editor_memory` 2000 | 0.8956 | 0.7832 | 0.9064 | 0.8051 | +0.0008 | 0.0013 | +0.0027 | 0.0033 |
| `editor_memory` 4000 | 0.8916 | 0.7707 | 0.9060 | 0.8042 | −0.0031 | 0.0018 | −0.0079 | 0.0042 |
| `editor_memory` −1000 | 0.8939 | 0.7808 | 0.9057 | 0.8011 | −0.0009 | 0.0008 | +0.0002 | 0.0018 |
| 1000, `ed_gate` 0.2 | 0.8950 | 0.7822 | 0.9085 | 0.8109 | +0.0002 | 0.0007 | +0.0016 | 0.0026 |
| 1000, `ed_gate` 0.5 | 0.8953 | 0.7827 | 0.9085 | 0.8107 | +0.0005 | 0.0007 | +0.0021 | 0.0027 |
| **1250, `ed_gate` 0.2 — the winner** | **0.8963** | **0.7834** | **0.9084** | **0.8108** | **+0.0015** | 0.0010 | **+0.0028** | 0.0031 |
| 1250, `ed_gate` 0.1 | 0.8960 | 0.7837 | 0.9088 | 0.8111 | +0.0012 | 0.0010 | +0.0032 | 0.0028 |
| 1250, `ed_gate` 0.3 | 0.8957 | 0.7828 | 0.9084 | 0.8105 | +0.0009 | 0.0009 | +0.0022 | 0.0030 |
| 1250, `ed_gate` 0.2, `ed_min_n` 3 | 0.8963 | 0.7829 | 0.9079 | 0.8105 | +0.0015 | 0.0010 | +0.0024 | 0.0028 |
| 1500, `ed_gate` 0.2 | 0.8962 | 0.7827 | 0.9072 | 0.8063 | +0.0015 | 0.0010 | +0.0021 | 0.0033 |
| 1500, `ed_gate` 0.3 | 0.8959 | 0.7832 | 0.9073 | 0.8082 | +0.0012 | 0.0010 | +0.0026 | 0.0031 |
| 1750, `ed_gate` 0.2 | 0.8958 | 0.7822 | 0.9077 | 0.8081 | +0.0011 | 0.0012 | +0.0016 | 0.0033 |
| 2000, `ed_gate` 0.2 | 0.8953 | 0.7816 | 0.9078 | 0.8086 | +0.0005 | 0.0013 | +0.0010 | 0.0034 |
| 4000, `ed_gate` 0.2 | 0.8923 | 0.7721 | 0.9058 | 0.8030 | −0.0024 | 0.0017 | −0.0085 | 0.0045 |
| 1000, `ed_min_n` 1 / 3 / 6 | 0.8938 / 0.8957 / 0.8950 | 0.7791 / 0.7828 / 0.7810 | 0.9084 / 0.9075 / 0.9071 | 0.8119 / 0.8096 / 0.8077 | −0.0010 / +0.0009 / +0.0003 | 0.0006 | −0.0015 / +0.0022 / +0.0004 | 0.0026 |
| 1000 with `m_fixed` 1.1 | 0.8956 | 0.7842 | 0.9081 | 0.8100 | +0.0009 | 0.0008 | +0.0036 | 0.0026 |
| 1000 with `kappa_s` −250 | 0.8950 | 0.7816 | 0.9077 | 0.8109 | +0.0003 | 0.0007 | +0.0011 | 0.0029 |

Lever by lever.

1. **Chemistry is the worst-behaved thing measured in three rounds.** As a
   forecast bonus it is negative at every strength and both signs, −0.0003 at 50
   points and −0.0402 at 400, and inside the likelihood it is worse still,
   −0.0314 at `chem_u = 0.2`. Section 1.1 says why and says it without a number:
   `T(L)` marks a lineup that can beat its forecast *and* a lineup that can miss
   it, so a linear term in it is asked to push in two directions at once and
   pushes in neither. Doing it at the question level — the version the round-2
   agent singled out as the most promising thing left — is the worse half.
2. **One global `m` has an interior optimum at 1.1 and it is worth nothing.**
   +0.0004 ± 0.0005 on tune, exactly 0.0000 on test. The curve is smooth and
   single-peaked — 0.9 is −0.0004, 1.05 is +0.0003, 1.15 is −0.0004 — so the
   question is settled rather than unresolved: the collaboration exponent is
   about 1.1 and being right about that buys half a standard error. **The
   difficulty-aware forecast loses at every `m`**, including at `m = 1` where it
   is a pure reparameterisation of the ranking, −0.0023 ± 0.0013. Ranking a
   field by expected answers rather than by lineup strength discards the part of
   `Q(L)` that the tempering of DESIGN 4.1 puts there on purpose.
3. **Training on the unrated tournaments in the honest form is neutral on tune
   and negative on test.** The players-only form the plan describes loads **6731
   tournaments against 5605** — 1126 extra — and the metric still covers exactly
   650 tune and 284 test fields, so the comparison is like for like, which is
   what round 2 could not say of `include_unrated`. At `w_unrated` 0.25 and 0.5 it
   is +0.0002 on tune and −0.0023 / −0.0037 on test; at 1.0 it is −0.0051, which
   is round 2's number for the cruder version. **The tournaments the official
   system leaves out are left out for a reason**, and letting them move the
   players without letting them move the teams does not change that.
4. **The lineup-level quantile is the per-player one in different units.**
   `sd(Q(L))` correlates with mean `σ` at 0.932 (section 1.4), and the sweep is
   flat: +0.0002 to +0.0004 on tune at every `c`, −0.0008 to −0.0018 on test.
   Round 2's `sigma_v` had already answered this and the roster-variance version
   does not differ materially, exactly as the round-2 agent predicted.
5. **`a` is at 1 because the data puts it there.** Both directions lose and the
   larger the move the worse it gets: −0.0015 at 0.8, −0.0022 at 1.2, −0.0061 at
   1.5. Section 1.5's calibration table said so before the sweep.
6. **Series memory is positive on tune at the sign the residual says is
   wrong, and negative on test.** The residual check says momentum (+0.032), so
   `kappa_s > 0` should win; every positive `kappa_s` loses. `kappa_s = −500`,
   mean reversion, is +0.0013 ± 0.0007 on tune — 1.9 SE, the second-best single
   point of the round — and **−0.0010 ± 0.0007 on test**. The two windows
   disagree in sign, which is what section 1.6's fading correlation predicts and
   what noise looks like. Not shipped.
7. **The user's lever, both ways round.** In the question gradients (7a) every
   variant loses, and the pattern is the giveaway: the tighter the gate the less
   it loses — `gate` 0.5 is −0.0048, 0.2 is −0.0040, 0.05 is −0.0024, 0.02 is
   −0.0001, 0.01 is +0.0004 — and the limit of a tighter gate is **no affinity at
   all**, which is the baseline. The decisive test is the prior width: if strong
   players really carry a readable affinity, the few who pass a tight gate should
   want *more* room, not less. At `gate` 0.05 a wider prior (`s_aff` 0.6) is
   −0.0043, the worst of the family, and a narrower one (`s_aff` 0.15) is +0.0003,
   indistinguishable from off. **Scaling by strength is the same story**: κ 0.3
   and 0.5 are about as bad as flat affinity, κ 1 is −0.0064 and κ 2 is −0.0313.
   Section 1.7 says the effect is there; DESIGN 8.2 says why this estimator
   cannot see it, and the single online pass has now failed to identify the
   editor columns in three separate rounds and two parameterisations.

   In the outcomes (7b) the same hypothesis wins. Nineteen of the twenty-one
   settings measured are positive on the tune window and eleven of them on both
   windows, the optimum is interior in every direction, and the gate the residual
   check asks for is where the tune window puts it.

## 3. The winner

```
[team.s2] editor_memory = 1250.0   (new)
          ed_gate       = 0.2      (new)
          ed_min_n      = 2.0      (new)
```

Nothing else in `config.toml` changed, and the eight other switches this round
added — `question.m_fixed`, `a_in_person`, `chem_u`, `aff_strength` with
`aff_kappa`, `aff_s_max` and `aff_gate`, `data.train_unrated` with `w_unrated`,
`team.s2.q_sigma_c`, `chemistry`, `difficulty_forecast`, `kappa_s` and
`series_k` — ship at the value that changes nothing.

The rule, in words. For every pair of a player and an editor the engine keeps
the mean of `(forecast place − actual place) / n` over the in-person fields that
player has played under that editor. A lineup's forecast gains `editor_memory`
display points per unit of the mean of those over its members and today's
editors, counting only players in the top fifth of the established panel and only
pairs with at least two past results. It is advanced **after** a field is
forecast, so no result predicts itself, and it is rebuilt from empty on each
backfill pass, so nothing crosses from one pass to the next.

```
ndcg tune 0.8963 @3 0.7834  unweighted 0.9014 @3 0.7988  (650 tournaments)
ndcg test 0.9084 @3 0.8108  unweighted 0.9070 @3 0.8122  (284 tournaments)
paired tune minus round 2: +0.0015 se 0.0010  @3 +0.0028 se 0.0031
paired test minus round 2: +0.0007 se 0.0008  @3 +0.0035 se 0.0023
```

**What it does not touch.** The bonus is added to the forecast and to nothing
else: `Q(L)` is left alone, so the Elo step, the pull, the usual lineup and every
published rating are bit-identical to round 2, and it is read at an in-person
table only, because in-person residuals are what it is made of, so every
synchronous and strictly synchronous forecast is bit-identical too. That is not
an accident of tuning. The first version put the bonus inside `Q(L)`, which moved
the team scale enough to shift test 4 from 6.0% to 5.9% and cost 0.0006 of the
synchronous Spearman for nothing; moving it to the forecast alone left the
in-person NDCG unchanged to four decimals and gave those back. The display maps
were **not** refitted, because the team scale did not move.

**How big is it, honestly.** +0.0015 against a standard error of 0.0010 is 1.5
SE, and this is the best of 93 runs, which on round 2's own argument is about
what the maximum of 93 draws from noise gives. Three things separate it from
that. The gain has the **same sign on both windows and on both metrics**, four
cells out of four. The whole family is positive and smooth — nineteen of the
twenty-one settings measured, at four gates and four history thresholds, are
positive on tune and eleven of them on both windows — with an interior optimum in
every direction rather than a spike. And the mechanism was **written down
before the sweep**, from a residual correlation of +0.098 in the cohort the
switch then restricts itself to. Round 2's `teammate_shrink`, which shipped, had
+0.0006 ± 0.0011 and two agreeing windows and nothing else.

## 4. The winner against the field

`uv run tools/compare_ndcg.py`, test window, the 282 tournaments every database
forecasts, weights frozen.

| system | NDCG (weighted) | NDCG@3 (weighted) | NDCG | NDCG@3 |
|---|---:|---:|---:|---:|
| **round 3, the winner (player based)** | **0.9086** | **0.8111** | **0.9071** | **0.8125** |
| round 2, the winner (player based) | 0.9079 | 0.8076 | 0.9064 | 0.8091 |
| round 1, the merged model (player based) | 0.9067 | 0.8024 | 0.9050 | 0.8045 |
| rating.chgk.fun (player based) | 0.9040 | 0.7997 | 0.8983 | 0.7932 |
| chgk.quest (player based) | 0.9011 | 0.7914 | 0.8974 | 0.7883 |
| official chgk.gg (player based) | 0.8710 | 0.7261 | 0.8749 | 0.7421 |
| round 3, the winner (team based) | 0.8219 | 0.6592 | 0.8316 | 0.6791 |
| official chgk.gg (team based) | 0.7917 | 0.6123 | 0.7986 | 0.6244 |

Paired bootstrap over the same tournaments, 2000 resamples, weighted; the round-3
player row minus the other row.

| against | NDCG | SE | NDCG@3 | SE |
|---|---:|---:|---:|---:|
| round 2, the winner | +0.0007 | 0.0008 | +0.0035 | 0.0025 |
| round 1, the merged model | +0.0019 | 0.0016 | +0.0088 | 0.0043 |
| rating.chgk.fun | +0.0046 | 0.0035 | +0.0115 | 0.0080 |
| chgk.quest | +0.0075 | 0.0040 | +0.0198 | 0.0087 |
| official chgk.gg | +0.0376 | 0.0055 | +0.0851 | 0.0126 |
| round 3, team based | +0.0867 | 0.0089 | +0.1520 | 0.0173 |
| official chgk.gg, team based | +0.1169 | 0.0097 | +0.1989 | 0.0187 |

The round-3 team row is round 2's team row to four decimals, because the rating
did not move.

The two competitors on both windows, where rating.chgk.fun's mirror covers only
half the tune fields:

| against | window | NDCG | SE | NDCG@3 | SE | fields |
|---|---|---:|---:|---:|---:|---:|
| rating.chgk.fun | tune | +0.0118 | 0.0050 | +0.0300 | 0.0107 | 327 |
| chgk.quest | tune | +0.0033 | 0.0027 | +0.0096 | 0.0062 | 646 |
| rating.chgk.fun | test | +0.0046 | 0.0035 | +0.0115 | 0.0080 | 282 |
| chgk.quest | test | +0.0075 | 0.0040 | +0.0198 | 0.0087 | 282 |

**Still not three standard errors, and the plan's stopping rule was again not
reached.** The four cells are 2.4, 1.2, 1.3 and 1.9 SE, against round 2's 2.2,
0.7, 1.1 and 1.7. Every one of them moved the right way, and the smallest lead in
the table — chgk.quest on the tune window — nearly doubled, from +0.0018 ± 0.0027
to +0.0033 ± 0.0027. That is the shape of a real if small gain, and it is not the
shape of a settled argument.

## 5. The acceptance tests

`out/q2/accept_final.md` beside `out/q3/accept.md`, both on the frozen mirror
with the history written.

| | round 2 | round 3 | |
|---|---:|---:|---|
| test 1, cross-fitted log-loss, in person, all seasons | 0.4899 | 0.4899 | passed |
| test 1, the same in 2025/26 | 0.5160 | 0.5160 | passed |
| test 1, all formats, all seasons | 0.4853 | 0.4853 | passed |
| test 2, Spearman, in person | 0.8527 | **0.8528** | passed |
| test 2, Spearman, synchronous | 0.8182 | 0.8182 | passed |
| test 2, Spearman, strictly synchronous | 0.8187 | 0.8187 | passed |
| test 3, top 200 against chgk.quest | 121/200 | 121/200 | failed, unchanged |
| test 3, top 200 against rating.chgk.fun | 130/200 | 130/200 | failed, unchanged |
| test 3, Spearman over 50+ games, chgk.quest | 0.8556 | 0.8556 | |
| test 3, the same against rating.chgk.fun | 0.8028 | 0.8028 | |
| test 4, inflation, largest September deviation | 6.0% | 6.0% | passed |
| test 5, transfer invariance, teams moved | 0 (max delta 0) | 0 (max delta 0) | passed |
| test 5, results whose forecast changed | 280 227 | 280 227 | passed |
| test 6, winner rule, violations sole / shared | 0 / 0 | 0 / 0 | passed |
| test 7, team NDCG on its own, tune (w) | 0.8894 | 0.8894 | passed |
| test 7, the same, test window (w) | 0.9047 | 0.9047 | passed |
| forecast NDCG, test window (w) | 0.9077 | **0.9084** | |
| forecast NDCG@3, test window (w) | 0.8072 | **0.8108** | |
| naive Spearman, `mu` only, in person / sync | 0.7962 / 0.7523 | 0.7962 / 0.7523 | |
| naive Spearman, `x`, in person / sync | 0.8459 / 0.8205 | 0.8459 / 0.8205 | |
| innovation autocorrelation, in person | −0.053 | −0.053 | |
| innovation autocorrelation, all formats | 0.038 | 0.038 | |
| non-provisional players at the last release | 6192 | 6192 | |

Test 7 here is over the 596 tune and 268 test fields of `qualify.csv`, both
columns from this round's own runs; round 2's report quoted the narrower
four-system subset for the tune window, which is why its tune figure reads 0.8996
and this one 0.8894. The test figure, 0.9047, is the same number in both.

**Every row but one is identical, and that is the point.** The change moves the
forecast and cannot reach anything else — not the likelihood, not the rating, not
the published tables, not the synchronous forecast. It is the first thing either
of the last two rounds shipped whose blast radius is exactly the quantity being
optimised.

## 6. The tables

`uv run tools/q_ndcg_tops.py out/q2/final.db out/q3/final.db`, release 714 of
2026-09-03.

### Top 20 teams

| # | round 2 | round 3 |
|---:|---|---|
| 1 | Борский корабел 9704 | Борский корабел 9704 |
| 2 | Gay Guerrilla 8761 | Gay Guerrilla 8761 |
| 3 | Мир 8531 | Мир 8531 |
| 4 | Призраки Коши 8449 | Призраки Коши 8449 |
| 5 | Первая сборная 8394 | Первая сборная 8394 |
| 6 | ff 8211 | ff 8211 |
| 7 | Заячий тулупчик 8114 | Заячий тулупчик 8114 |
| 8 | Тихие Тигры 8027 | Тихие Тигры 8027 |
| 9 | Рабочее название 7860 | Рабочее название 7860 |
| 10 | Я просто медленно пишу 7804 | Я просто медленно пишу 7804 |
| 11 | Механизм реакции 7796 | Механизм реакции 7796 |
| 12 | Железные рукава 7786 | Железные рукава 7786 |
| 13 | Приматы 7697 | Приматы 7697 |
| 14 | Осторожно, надвигается кайфун 7591 | Осторожно, надвигается кайфун 7591 |
| 15 | Кузинатра 7576 | Кузинатра 7576 |
| 16 | Олеги Литвы 7550 | Олеги Литвы 7550 |
| 17 | Кошка или сова 7472 | Кошка или сова 7472 |
| 18 | 4 коња дебела 7453 | 4 коња дебела 7453 |
| 19 | Работяги Лайт 7424 | Работяги Лайт 7424 |
| 20 | Хронически разумные United 7407 | Хронически разумные United 7407 |

### Top 20 players

| # | round 2 | round 3 |
|---:|---|---|
| 1 | Руссо Максим 15847 | Руссо Максим 15847 |
| 2 | Семушин Иван 15393 | Семушин Иван 15393 |
| 3 | Васильев Дмитрий 15370 | Васильев Дмитрий 15370 |
| 4 | Мокин Игорь 14958 | Мокин Игорь 14958 |
| 5 | Миротин Евгений 14829 | Миротин Евгений 14829 |
| 6 | Изъюров Константин 14498 | Изъюров Константин 14498 |
| 7 | Тух Вадим 14364 | Тух Вадим 14364 |
| 8 | Брутер Александра 14356 | Брутер Александра 14356 |
| 9 | Коробейников Александр 14323 | Коробейников Александр 14323 |
| 10 | Галиакберов Денис 14283 | Галиакберов Денис 14283 |
| 11 | Гилёв Алексей 14243 | Гилёв Алексей 14243 |
| 12 | Халиуллин Рузель 14220 | Халиуллин Рузель 14220 |
| 13 | Фёдорова Татьяна 14180 | Фёдорова Татьяна 14180 |
| 14 | Марков Александр 14100 | Марков Александр 14100 |
| 15 | Сахаров Дмитрий 14082 | Сахаров Дмитрий 14082 |
| 16 | Карякин Дмитрий 13943 | Карякин Дмитрий 13943 |
| 17 | Комар Наталья 13939 | Комар Наталья 13939 |
| 18 | Онскуль Илья 13812 | Онскуль Илья 13812 |
| 19 | Разумов Юрий 13751 | Разумов Юрий 13751 |
| 20 | Плотников Дмитрий 13740 | Плотников Дмитрий 13740 |

**Both tables are byte-identical**, teams and players, all twenty rows and every
display value. The editor memory is read by the forecast and by nothing else, so
there is nothing for it to move here. Round 2 moved fourteen of the twenty
players; this round moves none, and that is the correct outcome rather than a
disappointing one.

## 7. What was not done, and what got worse

- **The margin over rating.chgk.fun and chgk.quest is still not three standard
  errors.** It is 1.3 and 1.9 on test and 2.4 and 1.2 on tune, against round 2's
  1.1, 1.7, 2.2 and 0.7. All four cells improved and none of them reached the
  plan's stopping rule. On the evidence of section 1, a fourth round of levers
  keyed to the residual will not get there: six of the seven mechanisms checked
  came back at or under 0.03, and the seventh — the one that came back at
  0.098 — has now been spent.
- **Six of the plan's levers lost**, five of them predictably and one of them
  (chemistry) very badly indeed. The single global `m` is the only one with an
  interior optimum worth recording at all, and it is worth +0.0004 ± 0.0005.
- **Series memory wins the tune window at the wrong sign and loses the test
  window.** `kappa_s = −500` is +0.0013 ± 0.0007 on tune and −0.0010 ± 0.0007 on
  test. It is the clearest example in the round of what the two-window protocol
  is for: read only the tune column and it is the second-best row in the table.
- **The editor affinity in the question gradients failed for the third round
  running**, now with the strength weighting the user asked for, in both the
  scaled and the gated form and at two ranks. The residual says the effect is
  there and this estimator cannot reach it; the prior-width probe (section 2,
  lever 7a) is the experiment that separates those two statements, and it comes
  down firmly on the estimator.
- **The difficulty-aware forecast loses even at `m = 1`**, where it is only a
  different monotone summary of the same lineup. That is worth flagging: the
  tempered roster estimate is not just a proxy for expected answers, it is a
  better ordering than expected answers, and the temperature `τ = 0.7` is doing
  work that a package-level expectation undoes.
- **Nothing got worse.** The one acceptance number that moved, in-person
  forecast Spearman, moved up by 0.0001. This is the first round of the three in
  which no number was made worse on purpose or by accident.
- **The weights were not re-derived.** Still frozen at `fffb987`, as the protocol
  requires.
- **`out/shipped.db` is no longer the pre-plan-3 model.** The live
  `out/ratings.db` has been rebuilt with round 2's configuration since round 2
  wrote its report, so that row scores 0.9079 / 0.8076 rather than round 2's
  quoted 0.8977 / 0.7799. It is replaced in section 4 by round 1's merged model,
  which keeps the table at eight rows and all of them meaningful.
- **`tools/q_mech.py` carries one unused helper**, `pair_counts`, the
  non-incremental version of the chemistry counter that the incremental one in
  `lever1` replaced. Left in place; it is the readable statement of what the
  incremental version computes.
