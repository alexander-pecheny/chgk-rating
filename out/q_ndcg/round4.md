# Plan 7: the question model, fourth round — the metric's own shape, and geography

Round 4 measured six levers over **93 fits** and ships none of them. The model
that comes out of it is the model that went in, bit for bit: the same
`config.toml` constants, the same 0.9086 / 0.8111 on the test window, the same
acceptance row to the last digit, the same top twenty teams and players.

That is the result and not a failure of the round. Four of the six levers have a
residual behind them, one of them at four standard errors, and the round's real
finding is **why a residual that size buys nothing in this metric**. Two answers
came back, and they are different:

- **The order already knows.** Home-and-away separates a team at home from a
  visitor by 0.0101 ± 0.0023 of a field — the largest cohort split any of four
  rounds has found — and the best any linear bonus on it could buy, with the
  coefficient chosen *in hindsight on the window it is scored on*, is 0.8966
  against 0.8963. A level effect of a hundredth of a field cannot move a top-four
  ordering that is already right most of the time.
- **The estimate is not calibrated for the use.** Ordering by `E[1/place]` is the
  Bayes rule for this metric under a correct posterior. Re-ordering the tune
  window offline has an interior maximum at **half** the propagated `sd(Q(L))`,
  worth +0.0002; at the sd's face value it is negative. The lineup's uncertainty
  is about twice what an ordering can use, and once it is halved there is nothing
  left to win.

And one answer nobody asked for: **the sampled order damages the editor memory it
feeds**. In a fit the ordering costs 0.0006 of tune NDCG; with the memory turned
off on both sides it costs 0.0001. The other 0.0005 is the memory's own
residuals, which are measured against the order the forecast produced.

The lead over rating.chgk.fun and chgk.quest is **exactly what round 3 left it**:
+0.0046 ± 0.0035 and +0.0075 ± 0.0040 on test, +0.0118 ± 0.0050 and
+0.0033 ± 0.0027 on tune. Still not three standard errors, and this round did not
move it by a digit.

## 0. The baseline

Everything is on the frozen mirror
`/mnt/HC_Volume_106816389/build/herdr-worktrees/r2out/buff_frozen.db`, symlinked
as `buff.db`. Round 3 refitted from `config.toml` at `1f0e06f` with
`--to 2026-09-05 --rows-every 0`:

```
ndcg tune 0.8963 @3 0.7834  unweighted 0.9014 @3 0.7988  (650 tournaments)
ndcg test 0.9084 @3 0.8108  unweighted 0.9070 @3 0.8122  (284 tournaments)
```

which is `out/q_ndcg/round3.md` in every digit. After the round's switches were
built, the same config on the same mirror gives the same eight numbers again, so
every one of them ships at the value that changes nothing. Protocol unchanged:
tune 2018-09-01 to 2024-08-29 decides, test 2024-08-30 to 2026-08-27 is read once
per row, weights frozen at `fffb987`, paired bootstrap of 2000 resamples against
round 3. `out/q_ndcg/results_round4.tsv`, driven from `out/q_ndcg/batch4/`.

**One correction to the diagnosis object.** `tools/q_mech.py` scores
`(forecast rank − actual place) / n` against the field it *forecasts*, which in a
field of 120 teams of which six carry a lineup is a rank on one side and a place
out of 120 on the other, so a handful of rows come back near −19. Round 3 read
that object only through weighted correlations and never printed a cohort mean,
so its numbers stand; this round prints cohort means, so `tools/q_mech4.py`
re-derives the residual from ranks on both sides. Under the corrected residual
round 3's headline correlation reads +0.065 over every lineup with a memory and
+0.038 in the strongest twentieth, against the +0.013 and +0.098 round 3 quotes —
the effect is still there and the strength gradient is weaker than it looked.

## 1. The mechanisms, checked before the sweeps

`uv run tools/q_mech4.py out/q4/base.db --lever N`, 17 392 results in 581
weighted in-person tune fields. The whole table is `out/q4/mech.md`.

### 1.1 Does the spread of a lineup buy first places (lever 1)

NDCG pays `1/place`, which is convex, so two lineups of the same mean strength
are not worth the same to it. Among the four lineups each field's forecast puts
highest, in fields of ten or more, split at the median of `sd(Q(L))`:

| field | lineup spread | results | won | 1/place | mean residual |
|---|---|---:|---:|---:|---:|
| tight (1st-to-4th gap in the lowest third) | narrow | 359 | 0.203 ± 0.021 | 0.410 | −0.0115 |
| tight | **wide** | 357 | **0.241 ± 0.023** | **0.436** | −0.0070 |
| middle | narrow | 357 | 0.297 ± 0.024 | 0.502 | +0.0116 |
| middle | wide | 355 | 0.172 ± 0.020 | 0.398 | −0.0060 |
| loose (the highest third) | narrow | 355 | 0.318 ± 0.025 | 0.532 | +0.0034 |
| loose | wide | 353 | 0.173 ± 0.020 | 0.407 | +0.0027 |

**The premise holds exactly where the plan said it would and nowhere else.** In a
bunched field the wider lineup wins 0.241 against 0.203 and takes more relevance,
0.436 against 0.410. In a field with a clear favourite it wins half as often,
0.173 against 0.318 — because there a wide lineup is not a lineup that might be
very good, it is one nobody has measured. `corr(sd, 1/place)` is +0.034 among the
top four of the tightest third and −0.075 among the top four overall.

### 1.2 Home and away (lever 2)

`tournaments.town_id` is a town id and `tournament_results.team_current_town` a
town name, and the mirror carries no map between them, so `data/towns.csv` is
fetched once from the old system's API. A player has no town at all; theirs is
the town they have played out of most often in the results **before** today.
17 370 of 17 392 results have a town on both sides and 46.6% of them are at home.

| cohort | results | mean residual | won |
|---|---:|---:|---:|
| the team's own town, **at home** | 8092 | **−0.0054 ± 0.0017** | 0.034 |
| the team's own town, **away** | 9278 | **+0.0047 ± 0.0015** | 0.038 |
| at home, in the strongest fifth of fields | 1931 | −0.0059 ± 0.0031 | 0.018 |
| away, in the strongest fifth of fields | 3128 | +0.0037 ± 0.0024 | 0.028 |
| at home, among the four highest forecasts | 1061 | −0.0091 ± 0.0036 | 0.224 |
| away, among the four highest forecasts | 1259 | +0.0034 ± 0.0028 | 0.249 |

**The plan's sign is backwards and the effect is the strongest cohort split of
four rounds.** The gap is 0.0101 ± 0.0023, 4.4 SE, and it survives in the
strongest fifth of fields and among the four highest forecasts, which is where
the round-2 diagnosis says the residual lives. The lineup reading agrees:

| share of the lineup from the town | results | mean residual | won |
|---|---:|---:|---:|
| none | 8042 | +0.0056 ± 0.0016 | 0.039 |
| under a half | 1015 | −0.0029 ± 0.0045 | 0.041 |
| a half or more | 2358 | −0.0001 ± 0.0030 | 0.050 |
| all of it | 5505 | −0.0044 ± 0.0020 | 0.028 |

weighted `corr(share of the lineup from the town, residual)` −0.031.

The mechanism reads the same way in both: **travelling is a choice a team in form
makes, and turning out at home is what everyone does.** A lineup that got on a
train is a lineup someone assembled on purpose; the local field is whoever was
free that Saturday. The model sees the six names and not the decision behind them.

### 1.3 Load (lever 3)

| feature | weighted corr with the residual | results |
|---|---:|---:|
| in-person tournaments per player, last 7 days | +0.007 | 17 390 |
| the same, last 30 days | +0.023 | 17 390 |
| days the event itself spans | +0.001 | 17 392 |

| mean in-person tournaments per player in the last 7 days | results | mean residual |
|---|---:|---:|
| 0 | 15 201 | −0.0006 ± 0.0012 |
| up to 1 | 2019 | +0.0045 ± 0.0031 |
| 1 to 2 | 166 | −0.0003 ± 0.0174 |
| over 2 | 4 | +0.0991 ± 0.0983 |

Nothing, and what little there is has the wrong sign for fatigue: a player who
played last week does slightly *better*, which is selection rather than rest.
A multi-day event is flat to four decimals — one day −0.0000, two +0.0000, three
or more +0.0002.

### 1.4 Season timing (lever 4)

| month | results | mean residual | mean theta of the lineup |
|---|---:|---:|---:|
| 08 | 555 | −0.0003 ± 0.0061 | 0.293 |
| 09 | 1368 | −0.0001 ± 0.0039 | 0.126 |
| 10 | 2034 | +0.0001 ± 0.0031 | 0.003 |
| 11 | 1769 | −0.0001 ± 0.0038 | 0.034 |
| 12 | 1527 | −0.0000 ± 0.0039 | −0.120 |
| 01 | 799 | −0.0001 ± 0.0053 | 0.094 |
| 02 | 1926 | +0.0001 ± 0.0035 | −0.124 |
| 03 | 1971 | −0.0000 ± 0.0032 | −0.080 |
| 04 | 1700 | +0.0001 ± 0.0035 | −0.011 |
| 05 | 1677 | −0.0001 ± 0.0034 | 0.106 |
| 06 | 998 | +0.0001 ± 0.0042 | 0.222 |
| 07 | 1068 | +0.0001 ± 0.0043 | 0.177 |

**Largest monthly deviation 0.0003 against a typical standard error of 0.0038.**
The plan gates this lever on the check showing a pattern above noise and it shows
the flattest table in four rounds, so **no switch was built and no fit was
spent**. August rust and May fatigue are real to the people who play and they are
not in the residual; what the months do carry is the *strength* of who turns out
— 0.293 in August against −0.124 in February — and the forecast already reads
that off the lineup.

### 1.5 The editor memory, on four keys (lever 5)

| key | all lineups | the strongest fifth | the strongest twentieth | results |
|---|---:|---:|---:|---:|
| player × editor (round 3's) | +0.065 | +0.013 | +0.038 | 8834 |
| team × editor | +0.062 | +0.011 | +0.035 | 6078 |
| player × the editors' set | −0.038 | n/a | n/a | **53** |
| player × editor, half-life 2 years | +0.078 | +0.019 | **+0.093** | 5958 |
| player × editor, half-life 4 years | +0.081 | +0.034 | +0.070 | 6557 |

50.8% of the results have a player × editor memory, 34.9% a team × editor one and
**0.3%** an editors'-set one: the exact pair of co-writers almost never repeats
twice with the same player, so that key is dead before it is measured. The team
key is the player key's equal on a third fewer results. **A half-life looks
better than no half-life on every cohort**, which is the one place in the round
where the residual check asks for something round 3 did not do.

### 1.6 Should the quantile depend on the field (lever 6)

| field, by the 1st-to-4th forecast gap | gap | results | corr(mean sigma, residual) |
|---|---:|---:|---:|
| tight (lowest third) | 475 | 6679 | −0.040 |
| middle | 960 | 6329 | −0.039 |
| loose (highest third) | 1841 | 4382 | −0.038 |

Flat over the whole field. Among the four highest forecasts, where the quantile
actually decides an order:

| field | results | corr(mean sigma, residual) | corr(mean sigma, 1/place) |
|---|---:|---:|---:|
| tight | 776 | **+0.026** | +0.030 |
| middle | 776 | −0.077 | −0.130 |
| loose | 772 | −0.068 | −0.243 |

The sign turns over between a bunched top and a clear favourite: at the top of a
tight field an uncertain lineup beats its forecast, and everywhere else it falls
short. That says `sigma_c` should be **smaller** in a tight field and larger in a
loose one, and it is the same sentence as lever 1's table read through the mean
instead of through the spread.

## 2. The levers

`d` is the paired difference against **round 3** with its bootstrap standard
error, tune window, 2000 resamples. Full table `out/q_ndcg/results_round4.tsv`.

| lever | tune NDCG | tune @3 | test NDCG | test @3 | d NDCG | SE | d @3 | SE |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **round 3, the baseline** | 0.8963 | 0.7834 | 0.9084 | 0.8108 | - | - | - | - |
| **1. order by expected relevance** | | | | | | | | |
| `erel` | 0.8949 | 0.7819 | 0.9081 | 0.8088 | −0.0014 | 0.0006 | −0.0015 | 0.0012 |
| `erel`, `sigma_c` 0.75 | 0.8945 | 0.7784 | 0.9073 | 0.8080 | −0.0018 | 0.0012 | −0.0050 | 0.0024 |
| `erel`, `sigma_c` 0 | 0.8918 | 0.7710 | 0.9052 | 0.8020 | −0.0045 | 0.0014 | −0.0124 | 0.0034 |
| `p_first` | 0.8948 | 0.7801 | 0.9074 | 0.8064 | −0.0015 | 0.0006 | −0.0033 | 0.0020 |
| `p_first`, `sigma_c` 0.75 | 0.8931 | 0.7749 | 0.9056 | 0.8030 | −0.0032 | 0.0011 | −0.0085 | 0.0030 |
| `p_first`, `sigma_c` 0 | 0.8862 | 0.7576 | 0.9017 | 0.7927 | −0.0101 | 0.0016 | −0.0258 | 0.0040 |
| `erel_d` | 0.8920 | 0.7751 | 0.9044 | 0.7957 | −0.0043 | 0.0014 | −0.0083 | 0.0034 |
| `erel_d`, `sigma_c` 0.75 | 0.8919 | 0.7739 | 0.9057 | 0.8004 | −0.0044 | 0.0015 | −0.0096 | 0.0036 |
| `erel_d`, `sigma_c` 0 | 0.8895 | 0.7656 | 0.9058 | 0.8035 | −0.0068 | 0.0017 | −0.0179 | 0.0044 |
| the quantiles alone: `sigma_c` 0.75 / 0 | 0.8951 / 0.8920 | 0.7796 / 0.7714 | 0.9072 / 0.9060 | 0.8061 / 0.8028 | −0.0012 / −0.0043 | 0.0010 | −0.0038 / −0.0120 | 0.0021 |
| `erel`, only fields under a 600-point gap | 0.8952 | 0.7820 | 0.9086 | 0.8101 | −0.0011 | 0.0005 | −0.0015 | 0.0010 |
| `erel`, under 1000 | 0.8954 | 0.7822 | 0.9084 | 0.8093 | −0.0009 | 0.0005 | −0.0012 | 0.0011 |
| **`erel`, under 300** | **0.8960** | 0.7827 | 0.9085 | 0.8094 | **−0.0003** | 0.0002 | −0.0007 | 0.0005 |
| `p_first`, under 600 | 0.8955 | 0.7825 | 0.9078 | 0.8078 | −0.0008 | 0.0004 | −0.0009 | 0.0017 |
| `erel`, 1000 draws | 0.8955 | 0.7822 | 0.9087 | 0.8097 | −0.0008 | 0.0005 | −0.0012 | 0.0011 |
| `erel`, 1000 draws, `rank_sd` 0.25 | 0.8960 | 0.7829 | 0.9084 | 0.8106 | −0.0003 | 0.0004 | −0.0005 | 0.0005 |
| `erel`, 1000 draws, `rank_sd` 0.5 | 0.8957 | 0.7827 | 0.9083 | 0.8107 | −0.0006 | 0.0004 | −0.0007 | 0.0008 |
| `erel`, 1000 draws, `rank_sd` 0.75 | 0.8955 | 0.7830 | 0.9084 | 0.8110 | −0.0008 | 0.0004 | −0.0004 | 0.0009 |
| `erel`, `rank_sd` 0.5, under 1000 | 0.8958 | 0.7825 | 0.9083 | 0.8107 | −0.0005 | 0.0004 | −0.0009 | 0.0006 |
| the memory off, no sampling | 0.8948 | 0.7806 | 0.9077 | 0.8072 | −0.0015 | 0.0010 | −0.0028 | 0.0031 |
| the memory off, `erel` at `rank_sd` 0.5 | 0.8947 | 0.7799 | 0.9076 | 0.8072 | −0.0016 | 0.0010 | −0.0036 | 0.0032 |
| **2. home and away** | | | | | | | | |
| `home_bonus` −50 | 0.8964 | 0.7821 | 0.9075 | 0.8076 | +0.0001 | 0.0005 | −0.0013 | 0.0012 |
| `home_bonus` −100 | 0.8953 | 0.7801 | 0.9073 | 0.8060 | −0.0010 | 0.0008 | −0.0033 | 0.0018 |
| `home_bonus` −200 | 0.8948 | 0.7801 | 0.9073 | 0.8057 | −0.0015 | 0.0010 | −0.0033 | 0.0021 |
| `home_bonus` −400 | 0.8934 | 0.7765 | 0.9059 | 0.7990 | −0.0029 | 0.0015 | −0.0069 | 0.0032 |
| `home_bonus` +100 | 0.8968 | 0.7828 | 0.9073 | 0.8097 | +0.0005 | 0.0007 | −0.0006 | 0.0016 |
| `home_bonus` +200 | 0.8949 | 0.7797 | 0.9072 | 0.8096 | −0.0014 | 0.0012 | −0.0038 | 0.0023 |
| the whole lineup local, −100 / −200 / −400 | 0.8956 / 0.8949 / 0.8940 | 0.7821 / 0.7802 / 0.7778 | 0.9087 / 0.9088 / 0.9083 | 0.8114 / 0.8096 / 0.8027 | −0.0007 / −0.0014 / −0.0023 | 0.0006 | −0.0014 / −0.0032 / −0.0056 | 0.0017 |
| the team's own town, −100 | 0.8961 | 0.7822 | 0.9077 | 0.8072 | −0.0002 | 0.0007 | −0.0012 | 0.0019 |
| the team's own town, −200 | 0.8947 | 0.7804 | 0.9056 | 0.8032 | −0.0016 | 0.0010 | −0.0030 | 0.0023 |
| the team's own town, −400 | 0.8945 | 0.7796 | 0.9038 | 0.7948 | −0.0018 | 0.0016 | −0.0038 | 0.0033 |
| the team's own town, +200 | 0.8943 | 0.7786 | 0.9068 | 0.8085 | −0.0020 | 0.0012 | −0.0048 | 0.0025 |
| **3. load** | | | | | | | | |
| `load_7` −50 / −100 / −200 | 0.8961 / 0.8960 / 0.8947 | 0.7827 / 0.7827 / 0.7802 | 0.9083 / 0.9076 / 0.9083 | 0.8107 / 0.8091 / 0.8085 | −0.0002 / −0.0003 / −0.0016 | 0.0003 | −0.0007 / −0.0007 / −0.0032 | 0.0007 |
| `load_7` +100 | 0.8959 | 0.7831 | 0.9081 | 0.8100 | −0.0004 | 0.0006 | −0.0004 | 0.0017 |
| `load_30` −25 / −50 | 0.8959 / 0.8955 | 0.7825 / 0.7817 | 0.9082 / 0.9081 | 0.8086 / 0.8082 | −0.0004 / −0.0008 | 0.0004 | −0.0009 / −0.0018 | 0.0007 |
| `load_30` +50 | 0.8959 | **0.7843** | 0.9076 | 0.8112 | −0.0004 | 0.0009 | +0.0009 | 0.0023 |
| **4. season timing** | not implemented — the check is 0.0003 against an SE of 0.0038 | | | | | | | |
| **5. the editor memory** | | | | | | | | |
| 750, gate 0.1 / 0.2 / 0.3 | 0.8948 / 0.8950 / 0.8951 | 0.7816 / 0.7817 / 0.7818 | 0.9084 / 0.9084 / 0.9083 | 0.8106 / 0.8111 / 0.8106 | −0.0015 / −0.0013 / −0.0012 | 0.0008 | −0.0018 / −0.0017 / −0.0017 | 0.0020 |
| 1000, gate 0.1 / 0.3 | 0.8956 / 0.8953 | 0.7840 / 0.7830 | 0.9085 / 0.9085 | 0.8104 / 0.8108 | −0.0007 / −0.0010 | 0.0008 | +0.0006 / −0.0004 | 0.0023 |
| 1250, gate 0.15 | 0.8960 | 0.7835 | 0.9083 | 0.8108 | −0.0003 | 0.0004 | +0.0001 | 0.0017 |
| 1400, gate 0.1 | 0.8960 | 0.7833 | 0.9074 | 0.8079 | −0.0003 | 0.0006 | −0.0001 | 0.0023 |
| **1450, gate 0.15** | **0.8968** | 0.7850 | 0.9074 | 0.8088 | **+0.0005** | 0.0004 | +0.0016 | 0.0016 |
| **1500, gate 0.1** | **0.8969** | 0.7847 | 0.9081 | 0.8095 | **+0.0006** | 0.0007 | +0.0013 | 0.0025 |
| 1500, gate 0.12 | 0.8958 | 0.7820 | 0.9078 | 0.8088 | −0.0005 | 0.0007 | −0.0015 | 0.0021 |
| **1500, gate 0.15** | **0.8968** | **0.7859** | 0.9072 | 0.8086 | **+0.0005** | 0.0004 | **+0.0024** | 0.0013 |
| 1500, gate 0.05 | 0.8951 | 0.7808 | 0.9070 | 0.8078 | −0.0012 | 0.0007 | −0.0026 | 0.0025 |
| 1550, gate 0.15 | 0.8962 | 0.7836 | 0.9077 | 0.8089 | −0.0001 | 0.0006 | +0.0002 | 0.0018 |
| 1600, gate 0.1 | 0.8965 | 0.7851 | 0.9082 | 0.8104 | +0.0002 | 0.0008 | +0.0017 | 0.0024 |
| 2000, gate 0.1 / 0.3 | 0.8956 / 0.8949 | 0.7839 / 0.7812 | 0.9059 / 0.9079 | 0.8062 / 0.8075 | −0.0007 / −0.0014 | 0.0010 | +0.0005 / −0.0023 | 0.0024 |
| `ed_min_n` 1 / 3 / 4 | 0.8948 / 0.8963 / 0.8953 | 0.7817 / 0.7829 / 0.7800 | 0.9075 / 0.9079 / 0.9068 | 0.8083 / 0.8105 / 0.8086 | −0.0015 / +0.0000 / −0.0010 | 0.0009 | −0.0017 / −0.0005 / −0.0034 | 0.0017 |
| half-life 2 years | 0.8953 | 0.7802 | 0.9076 | 0.8078 | −0.0010 | 0.0010 | −0.0032 | 0.0024 |
| half-life 4 years | 0.8956 | 0.7812 | 0.9077 | 0.8091 | −0.0007 | 0.0008 | −0.0022 | 0.0020 |
| half-life 2 / 4 at 2000 | 0.8959 / 0.8967 | 0.7807 / 0.7828 | 0.9062 / 0.9061 | 0.8043 / 0.8014 | −0.0004 / +0.0004 | 0.0011 | −0.0027 / −0.0007 | 0.0026 |
| team × editor, 1250 | 0.8944 | 0.7797 | 0.9081 | 0.8095 | −0.0019 | 0.0013 | −0.0037 | 0.0027 |
| team × editor, 2000 | 0.8931 | 0.7756 | 0.9079 | 0.8071 | −0.0032 | 0.0016 | −0.0078 | 0.0032 |
| team × editor, 3000 | 0.8911 | 0.7728 | 0.9057 | 0.7983 | −0.0052 | 0.0018 | −0.0107 | 0.0037 |
| the editors' set | 0.8947 | 0.7805 | 0.9078 | 0.8072 | −0.0016 | 0.0010 | −0.0029 | 0.0031 |
| `ed_phi` 0.05 / 0.1 / 0.25 | 0.8946 / 0.8946 / 0.8946 | 0.7799 / 0.7809 / 0.7804 | 0.9077 / 0.9078 / 0.9078 | 0.8072 / 0.8080 / 0.8092 | −0.0017 / −0.0017 / −0.0017 | 0.0010 | −0.0035 / −0.0025 / −0.0030 | 0.0030 |
| **6. a field-conditional quantile** | | | | | | | | |
| gap 600, tight 1.0 | 0.8959 | 0.7824 | 0.9084 | 0.8107 | −0.0004 | 0.0008 | −0.0010 | 0.0015 |
| gap 600, tight 0.75 | 0.8958 | 0.7813 | 0.9076 | 0.8078 | −0.0005 | 0.0009 | −0.0021 | 0.0018 |
| gap 600, loose 2.0 | 0.8958 | 0.7819 | 0.9090 | 0.8116 | −0.0005 | 0.0005 | −0.0015 | 0.0015 |
| gap 600, tight 1.0 loose 2.0 | 0.8955 | 0.7808 | 0.9089 | 0.8114 | −0.0008 | 0.0009 | −0.0026 | 0.0022 |
| gap 700, loose 2.0 | 0.8963 | 0.7838 | 0.9089 | 0.8115 | +0.0000 | 0.0004 | +0.0004 | 0.0011 |
| gap 1000, tight 1.0 | 0.8958 | 0.7817 | 0.9082 | 0.8096 | −0.0005 | 0.0009 | −0.0017 | 0.0017 |
| gap 1000, tight 0.75 | 0.8945 | 0.7788 | 0.9070 | 0.8068 | −0.0018 | 0.0010 | −0.0046 | 0.0020 |
| gap 1000, loose 1.75 | 0.8962 | 0.7833 | 0.9084 | 0.8104 | −0.0001 | 0.0002 | −0.0001 | 0.0003 |
| gap 1000, loose 2.0 | 0.8962 | 0.7832 | 0.9089 | 0.8116 | −0.0001 | 0.0002 | −0.0002 | 0.0008 |
| gap 1000, loose 2.5 | 0.8962 | 0.7832 | 0.9090 | 0.8118 | −0.0001 | 0.0003 | −0.0003 | 0.0011 |
| gap 1000, tight 1.0 loose 2.0 | 0.8956 | 0.7813 | 0.9086 | 0.8100 | −0.0007 | 0.0009 | −0.0021 | 0.0019 |
| **gap 1500, loose 2.0** | **0.8964** | **0.7839** | 0.9084 | 0.8109 | **+0.0001** | 0.0001 | +0.0005 | 0.0005 |
| the sign reversed: gap 600, tight 2.0 | 0.8956 | 0.7824 | 0.9073 | 0.8091 | −0.0007 | 0.0004 | −0.0010 | 0.0010 |
| the sign reversed: gap 1000, tight 2.0 loose 1.0 | 0.8952 | 0.7813 | 0.9070 | 0.8067 | −0.0011 | 0.0006 | −0.0021 | 0.0016 |
| the control: `sigma_c` 1.25 / 1.75 / 2.0 | 0.8958 / 0.8960 / 0.8954 | 0.7828 / 0.7825 / 0.7816 | 0.9087 / 0.9079 / 0.9080 | 0.8104 / 0.8094 / 0.8102 | −0.0005 / −0.0003 / −0.0009 | 0.0006 | −0.0006 / −0.0009 / −0.0018 | 0.0014 |
| **the combinations** | | | | | | | | |
| memory (1500, 0.15) + gap 1500 loose 2.0 | 0.8966 | 0.7858 | 0.9079 | 0.8102 | +0.0003 | 0.0004 | +0.0023 | 0.0014 |
| memory (1500, 0.15) + `erel` under 300 | 0.8959 | 0.7826 | 0.9076 | 0.8073 | −0.0004 | 0.0004 | −0.0008 | 0.0014 |
| gap 700 loose 2.0 + `erel` under 300 | 0.8960 | 0.7827 | 0.9090 | 0.8101 | −0.0003 | 0.0005 | −0.0007 | 0.0011 |
| all three | 0.8961 | 0.7836 | 0.9079 | 0.8088 | −0.0002 | 0.0006 | +0.0001 | 0.0014 |

Lever by lever.

1. **The metric-targeted ordering loses at every quantile, every rule and every
   number of draws, and the way it loses says why.** At `sigma_c` 1.5 it is
   −0.0014 ± 0.0006; restrict the sampling to fields whose top four are within
   1000 points and it is −0.0009, within 600 −0.0011, within 300 −0.0003. **The
   less of the lever is used the less it costs, and the limit of the restriction
   is no sampling at all** — the same shape round 3 found in the editor affinity,
   and the same conclusion.

   Two things were done to give it its best chance. Raising the draws from 200 to
   1000 halves the loss, from −0.0014 to −0.0008, so a third of it was the
   estimator's own noise and two thirds are real. And `tools/q_diag4.py` re-orders
   the tune window offline at a range of multipliers on `sd(Q(L))`:

   | multiplier on the sd | NDCG | NDCG@3 | fields whose order changed |
   |---|---:|---:|---:|
   | 0 (the mean ordering) | 0.8963 | 0.7834 | 45 |
   | 0.1 | 0.8963 | 0.7835 | 98 |
   | 0.25 | 0.8964 | 0.7836 | 173 |
   | **0.5** | **0.8965** | **0.7840** | 289 |
   | 1.0 | 0.8961 | 0.7841 | 391 |
   | 2.0 | 0.8957 | 0.7834 | 512 |

   **There is an interior maximum, and it is at half the propagated sd.** The
   mechanism is real and the lineup's uncertainty is about twice what an ordering
   can use — which is the same statement `sigma_c = 1.5`, a 1.5-sigma discount on
   a sigma the model itself computes, has been making since round 1. At the
   optimum it is worth +0.0002, a fifth of a standard error.

   Put that 0.5 into a fit and it comes out −0.0006 ± 0.0004 instead of +0.0002.
   The difference is not the ordering. **Turn the editor memory off on both sides
   and the sampled order costs 0.0001** — 0.8948 against 0.8947 — because the
   forecast order is also what the memory's residuals of DESIGN 4.8 are measured
   against, and a noisier order makes a worse memory. A forecast-side change in
   this model is no longer local.

   `erel_d`, which adds the pairwise noise of `D` as a Gumbel and so makes one
   draw a Plackett-Luce ranking at exactly the Elo scale the engine already uses,
   is the **worst** of the family at −0.0043. Adding the noise the model says is
   there makes the order worse: the pairwise scale `D = 1200` is fitted to
   outcomes, not to the residual uncertainty of a lineup estimate, and using it
   twice double-counts.

2. **Home and away buys nothing at either sign, at four strengths and two
   readings of "home".** The best row of the family is `home_bonus` +100 at
   +0.0005 ± 0.0007 — the sign the residual says is *wrong* — and it is −0.0011
   on test. The negative bonuses the residual asks for are −0.0010 at 100 points,
   −0.0015 at 200 and −0.0029 at 400.

   Why a 4.4-SE cohort split buys nothing is the round's most useful negative
   result, and `tools/q_diag4.py --what within` answers it. The feature is not
   constant inside a field — 52% to 57% of its variance is within-field, so there
   is something for a bonus to reorder. What there is not is enough of it:

   | feature | share of variance inside a field | best bonus | NDCG | @3 |
   |---|---:|---:|---:|---:|
   | the team's own town is the tournament's | 0.57 | +100 | 0.8966 | 0.7819 |
   | share of the lineup local to the town | 0.52 | +100 | 0.8965 | 0.7825 |
   | in-person tournaments per player, last 7 days | 0.32 | +100 | 0.8963 | 0.7836 |
   | the same, last 30 days | 0.48 | 0 | 0.8963 | 0.7834 |
   | sd of the lineup's own strength | 0.60 | −2 | 0.8963 | 0.7834 |

   The bonus in that table is chosen **in hindsight, on the very window the NDCG
   is read on**, and the ceiling is 0.8966 against 0.8963. A level effect of a
   hundredth of a field cannot move an ordering whose top four are already right;
   and the in-hindsight sign is positive, so even the 0.0003 is the wrong
   direction from the residual and therefore noise.

3. **Load is nothing, exactly as its check said.** Every row is between −0.0016
   and −0.0002 on tune, and the one row that is positive anywhere is `load_30`
   **+50** — more play, better forecast — at +0.0009 ± 0.0023 of tune NDCG@3,
   which is selection and not rest.

4. **Season timing was not built.** The check's largest monthly deviation is
   0.0003 against a standard error of 0.0038; the plan gates the lever on that
   check and it did not pass. Zero fits spent.

5. **The editor memory is where round 3 put it.** Forty-eight cells of
   `editor_memory` × `ed_gate` × `ed_min_n`, plus the half-life, plus two more
   keys, plus the `phi` channel, and nothing clears the plan's rule.

   The best tune cell is `(1500, 0.15)` at +0.0005 ± 0.0004 — 1.25 SE, so it
   passes the first half of the rule — and it is **−0.0013 ± 0.0009 on test**, so
   it fails the second. It is also a spike and not a ridge: its four nearest
   neighbours read +0.0005 at (1450, 0.15), −0.0001 at (1550, 0.15), −0.0005 at
   (1500, 0.12) and −0.0012 at (1500, 0.05). Round 3's shipped cell has smooth,
   positive neighbours in every direction and this one does not, which is the
   distinction round 3's own section 3 drew between a gain and the maximum of
   ninety draws.

   **The half-life is the round's clearest case of a residual check being
   overruled.** Forgetting reads better on every cohort of section 1.5 — +0.093
   in the strongest twentieth at two years against +0.038 without — and loses in
   the fit at both half-lives and both constants, −0.0010 and −0.0007. An editor
   a player read well in 2018 is an editor they still read well; what the decay
   throws away is sample size, and the memory is short of that before it is short
   of freshness.

   **The team key is the player key with a third of the evidence removed.** The
   residual likes it as much (+0.052 against +0.048 in the strongest fifth) and
   it covers 34.9% of the results against 50.8%; the fit reads −0.0019 at the
   player key's own constant and gets worse as the constant grows. **The editors'
   set is dead on coverage**: 53 results of 17 392, and it scores what the memory
   switched off scores.

   **The memory does not belong in `phi`.** At 0.05, 0.1 and 0.25 it is
   −0.0017 ± 0.0010, which is the number for the memory switched off entirely.
   Round 3 kept the term out of `Q(L)` to protect the acceptance row; this round
   shows that putting it there also destroys the term itself.

6. **The field-conditional quantile is a null with a clean control.** The best
   cell, a harsher quantile in a loose field at a 1500-point threshold, is
   +0.0001 ± 0.0001 on tune and 0.0000 on test: it is a rule that almost never
   fires and does nothing when it does. The reversed sign loses, −0.0007 and
   −0.0011, which is at least the right way round. And the control the lever
   needs — is a harsher quantile in a loose field anything more than a harsher
   quantile everywhere? — comes back at −0.0005, −0.0003 and −0.0009 for a global
   `sigma_c` of 1.25, 1.75 and 2.0. **`sigma_c = 1.5` is where the data puts it,
   and one number is enough.**

The combinations add nothing: the three least-bad rows together are
−0.0002 ± 0.0006, and the best pair is +0.0003 ± 0.0004, below the memory cell on
its own.

## 3. The winner

Round 3's, unchanged. No key of `config.toml` changed value. Ten new switches
were added and every one of them ships at the value that changes nothing:

```
[team.s2] rank_rule = "mean"   rank_draws = 200   rank_sd = 1.0   rank_gap = 0
          home_bonus = 0   home_min = 0.5   home_team_bonus = 0
          load_7 = 0   load_30 = 0
          ed_half_life = 0   ed_key = "player_editor"   ed_phi = 0
          tight_gap = 0   sigma_c_tight = 1.5   sigma_c_loose = 1.5
```

```
ndcg tune 0.8963 @3 0.7834  unweighted 0.9014 @3 0.7988  (650 tournaments)
ndcg test 0.9084 @3 0.8108  unweighted 0.9070 @3 0.8122  (284 tournaments)
paired tune minus round 3: 0.0000 se 0.0000  @3 0.0000 se 0.0000
paired test minus round 3: 0.0000 se 0.0000  @3 0.0000 se 0.0000
```

## 4. The winner against the field

`uv run tools/compare_ndcg.py`, test window, the 282 tournaments every database
forecasts, weights frozen.

| system | NDCG (weighted) | NDCG@3 (weighted) | NDCG | NDCG@3 |
|---|---:|---:|---:|---:|
| **round 4, the winner (player based)** | **0.9086** | **0.8111** | **0.9071** | **0.8125** |
| round 3, the winner (player based) | 0.9086 | 0.8111 | 0.9071 | 0.8125 |
| round 2, the winner (player based) | 0.9079 | 0.8076 | 0.9064 | 0.8091 |
| round 1, the merged model (player based) | 0.9067 | 0.8024 | 0.9050 | 0.8045 |
| rating.chgk.fun (player based) | 0.9040 | 0.7997 | 0.8983 | 0.7932 |
| chgk.quest (player based) | 0.9011 | 0.7914 | 0.8974 | 0.7883 |
| official chgk.gg (player based) | 0.8710 | 0.7261 | 0.8749 | 0.7421 |
| round 4, the winner (team based) | 0.8219 | 0.6592 | 0.8316 | 0.6791 |
| official chgk.gg (team based) | 0.7917 | 0.6123 | 0.7986 | 0.6244 |

Paired bootstrap over the same tournaments, 2000 resamples, weighted; the round-4
player row minus the other row.

| against | NDCG | SE | NDCG@3 | SE |
|---|---:|---:|---:|---:|
| round 3, the winner | +0.0000 | 0.0000 | +0.0000 | 0.0000 |
| round 2, the winner | +0.0007 | 0.0008 | +0.0035 | 0.0025 |
| round 1, the merged model | +0.0019 | 0.0016 | +0.0088 | 0.0043 |
| rating.chgk.fun | +0.0046 | 0.0035 | +0.0115 | 0.0080 |
| chgk.quest | +0.0075 | 0.0040 | +0.0198 | 0.0087 |
| official chgk.gg | +0.0376 | 0.0055 | +0.0851 | 0.0126 |
| round 4, team based | +0.0867 | 0.0089 | +0.1520 | 0.0173 |
| official chgk.gg, team based | +0.1169 | 0.0097 | +0.1989 | 0.0187 |

The round-3 row is zero to four decimals in every cell, which is the check that
this round changed nothing rather than a claim about it.

The two competitors on both windows, where rating.chgk.fun's mirror covers only
half the tune fields:

| against | window | NDCG | SE | NDCG@3 | SE | fields |
|---|---|---:|---:|---:|---:|---:|
| rating.chgk.fun | tune | +0.0118 | 0.0050 | +0.0300 | 0.0107 | 327 |
| chgk.quest | tune | +0.0033 | 0.0027 | +0.0096 | 0.0062 | 646 |
| rating.chgk.fun | test | +0.0046 | 0.0035 | +0.0115 | 0.0080 | 282 |
| chgk.quest | test | +0.0075 | 0.0040 | +0.0198 | 0.0087 | 282 |

**Still 2.4, 1.2, 1.3 and 1.9 standard errors, the same four numbers round 3
ended on.** The plan's stopping rule was not reached and was not approached.

## 5. The acceptance tests

`out/q4/accept.md` beside `out/q3/accept.md`, both on the frozen mirror with the
history written. The two files differ in one line, the path of the database they
were produced from.

| | round 3 | round 4 | |
|---|---:|---:|---|
| test 1, cross-fitted log-loss, in person, all seasons | 0.4899 | 0.4899 | passed |
| test 1, the same in 2025/26 | 0.5160 | 0.5160 | passed |
| test 1, all formats, all seasons | 0.4853 | 0.4853 | passed |
| test 2, Spearman, in person | 0.8528 | 0.8528 | passed |
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
| forecast NDCG, test window (w) | 0.9084 | 0.9084 | |
| forecast NDCG@3, test window (w) | 0.8108 | 0.8108 | |
| naive Spearman, `mu` only, in person / sync | 0.7962 / 0.7523 | 0.7962 / 0.7523 | |
| naive Spearman, `x`, in person / sync | 0.8459 / 0.8205 | 0.8459 / 0.8205 | |
| innovation autocorrelation, in person | −0.053 | −0.053 | |
| innovation autocorrelation, all formats | 0.038 | 0.038 | |
| non-provisional players at the last release | 6192 | 6192 | |

Test 5 was re-run from scratch — a second full history fit with
`--swap 28751:49804,16329:168@2018-12-22` and the paired eval — rather than
quoted, and it comes back at 0 teams moved with a maximum absolute difference of
exactly 0 and 280 227 forecasts changed. Test 7 was re-run over the 596 tune and
268 test base-roster fields and reproduces 0.8894 and 0.9047.

## 6. The tables

`uv run tools/q_ndcg_tops.py out/q3/final.db out/q4/final.db`, release 714 of
2026-09-03.

### Top 20 teams

| # | round 3 | round 4 |
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

| # | round 3 | round 4 |
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

Byte-identical, teams and players, all twenty rows and every display value,
because nothing shipped.

## 7. What was not done, and what got worse

- **Nothing shipped, and no number got worse.** Every acceptance row, every
  published rating and both comparison tables are what round 3 left them. The
  round cost 93 fits and bought a set of measurements rather than a model.
- **The margin over rating.chgk.fun and chgk.quest did not move.** 1.3 and 1.9 SE
  on test, 2.4 and 1.2 on tune, the same four cells as round 3. Four rounds and
  twenty-three levers have now been spent on it and the last two moved it by
  +0.0007 and 0.0000.
- **The plan's most promising lever had a real mechanism and still lost.** The
  ordering by expected relevance is the Bayes rule for this loss and the offline
  re-ranking finds its optimum at half the propagated sd, worth a fifth of a
  standard error. It does not survive contact with a fit, and most of what it
  loses there is a second-order effect on the editor memory rather than the
  ordering itself.
- **The largest residual signal of four rounds bought nothing.** Home against
  away is 4.4 SE on the residual and its in-hindsight ceiling on the metric is
  +0.0003, at the wrong sign. That is worth saying plainly: **the residual and
  the metric are not the same question**, and a round-5 plan that picks levers by
  residual size will find more of these.
- **The residual check was overruled twice.** The half-life reads better on every
  cohort and loses in every fit; the home effect reads at 4.4 SE and loses at
  every strength. In both cases the check was run first and written down first,
  which is what the protocol is for, and in both cases the sweep was the thing
  that decided.
- **A defect of round 3's wiring was found and left alone.** `fit.rs` hands the
  editor memory `(index in events, residual)` pairs and `EditorMemory::absorb`
  looks the team up as `t.teams[index]`; the two agree only when no result was
  skipped for having neither a lineup nor a rating. 91 of 58 803 in-person rated
  results have no roster, so at most 0.15% of the attributions can be wrong. It
  is not fixed here because fixing it would move the round-3 baseline this round
  is measured against; it is a one-line change for whoever re-baselines next.
- **`tools/q_mech.py`'s residual is wrong for cohort means** and was replaced in
  `tools/q_mech4.py` (section 0). Round 3's published correlations are unaffected;
  its section 1.7 cohort figures read differently under the corrected object,
  +0.038 in the strongest twentieth against +0.098.
- **`data/towns.csv` is the first file in the project fetched from the live API
  rather than the frozen mirror.** It is a static reference table of 1683 rows,
  it is committed, and it is read only by the geography switches, which are off.
- **The weights were not re-derived.** Still frozen at `fffb987`.
- **Season timing was measured and not implemented**, which saved about eight
  fits and is the one place the plan's own gate did its job.
