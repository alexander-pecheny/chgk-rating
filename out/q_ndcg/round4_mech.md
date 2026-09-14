# Round-4 mechanisms on the round-3 residual, 2018-09-01 to 2024-08-29

17392 results in 581 weighted in-person fields.

## Lever 1: does the spread of a lineup buy first places

Among the four lineups each field's forecast puts highest, by the tightness
of the field and the spread of the lineup. `won` is the share that finished
first; `1/place` is the relevance the metric pays.

| field | lineup spread | results | won | 1/place | mean residual |
|---|---|---:|---:|---:|---:|
| tight (1st-to-4th gap in the lowest third) | narrow | 359 | 0.203 ± 0.021 | 0.410 | -0.0115 |
| tight (1st-to-4th gap in the lowest third) | wide | 357 | 0.241 ± 0.023 | 0.436 | -0.0070 |
| middle | narrow | 357 | 0.297 ± 0.024 | 0.502 | +0.0116 |
| middle | wide | 355 | 0.172 ± 0.020 | 0.397 | -0.0060 |
| loose (the highest third) | narrow | 355 | 0.318 ± 0.025 | 0.532 | +0.0034 |
| loose (the highest third) | wide | 353 | 0.173 ± 0.020 | 0.407 | +0.0027 |

corr(sd, 1/place) over the four highest forecasts: -0.075; corr(sd, residual) -0.031; 2324 results.

corr(sd, 1/place) over the same, in the tightest third of fields: +0.034; corr(sd, residual) +0.040; 732 results.

corr(sd, 1/place) over every result: -0.268; corr(sd, residual) -0.040; 17390 results.

## Lever 2: home and away

17370 of 17392 results have a town on both sides; 46.6% of them are at home.

| cohort | results | mean residual | won |
|---|---:|---:|---:|
| the team's own town, at home | 8092 | -0.0054 ± 0.0017 | 0.034 |
| the team's own town, away | 9278 | +0.0047 ± 0.0015 | 0.038 |
| at home, in the strongest fifth of fields | 1931 | -0.0059 ± 0.0031 | 0.018 |
| away, in the strongest fifth of fields | 3128 | +0.0037 ± 0.0024 | 0.028 |
| at home, among the four highest forecasts | 1061 | -0.0091 ± 0.0036 | 0.224 |
| away, among the four highest forecasts | 1259 | +0.0034 ± 0.0028 | 0.249 |

16920 results have a town for at least one player. By the share of the
lineup that has played out of the tournament's own town before:

| share of the lineup from the town | results | mean residual | won |
|---|---:|---:|---:|
| none | 8042 | +0.0056 ± 0.0016 | 0.039 |
| under a half | 1015 | -0.0029 ± 0.0045 | 0.041 |
| a half or more | 2358 | -0.0001 ± 0.0030 | 0.050 |
| all of it | 5505 | -0.0044 ± 0.0020 | 0.028 |

weighted corr(share of the lineup from the town, residual): -0.031 over 16920 results.

## Lever 3: how much the players have already played

| feature | weighted corr with the residual | results |
|---|---:|---:|
| in-person tournaments per player, last 7 days | +0.007 | 17390 |
| the same, last 30 days | +0.023 | 17390 |
| days the event itself spans | +0.001 | 17392 |

| mean in-person tournaments per player in the last 7 days | results | mean residual |
|---|---:|---:|
| 0 | 15201 | -0.0006 ± 0.0012 |
| up to 1 | 2019 | +0.0045 ± 0.0031 |
| 1 to 2 | 166 | -0.0003 ± 0.0174 |
| over 2 | 4 | +0.0991 ± 0.0983 |

| days the event spans | results | mean residual |
|---|---:|---:|
| one | 7500 | -0.0000 ± 0.0018 |
| two | 8830 | +0.0000 ± 0.0014 |
| three or more | 1062 | +0.0002 ± 0.0044 |

## Lever 4: the month of the season

| month | results | mean residual | mean theta of the lineup |
|---|---:|---:|---:|
| 08 | 555 | -0.0003 ± 0.0061 | 0.293 |
| 09 | 1368 | -0.0001 ± 0.0039 | 0.126 |
| 10 | 2034 | +0.0001 ± 0.0031 | 0.003 |
| 11 | 1769 | -0.0001 ± 0.0038 | 0.034 |
| 12 | 1527 | -0.0000 ± 0.0039 | -0.120 |
| 01 | 799 | -0.0001 ± 0.0053 | 0.094 |
| 02 | 1926 | +0.0001 ± 0.0035 | -0.124 |
| 03 | 1971 | -0.0000 ± 0.0032 | -0.080 |
| 04 | 1700 | +0.0001 ± 0.0035 | -0.011 |
| 05 | 1677 | -0.0001 ± 0.0034 | 0.106 |
| 06 | 998 | +0.0001 ± 0.0042 | 0.222 |
| 07 | 1068 | +0.0001 ± 0.0043 | 0.177 |

Largest monthly deviation 0.0003 against a typical standard error of 0.0038.

## Lever 5: the editor memory, on four keys

| key | all lineups | the strongest fifth | the strongest twentieth | results |
|---|---:|---:|---:|---:|
| player x editor (round 3's) | +0.065 | +0.013 | +0.038 | 8834 |
| team x editor | +0.062 | +0.011 | +0.035 | 6078 |
| player x the editors' set | -0.038 | +nan | +nan | 53 |
| player x editor, half-life 2 years | +0.078 | +0.019 | +0.093 | 5958 |
| player x editor, half-life 4 years | +0.081 | +0.034 | +0.070 | 6557 |

50.8% of the results have a player x editor memory at all.

34.9% of the results have a team x editor memory at all.

0.3% of the results have a the editors' set memory at all.

## Lever 6: should the quantile depend on the field

| field, by the 1st-to-4th forecast gap | gap | results | corr(mean sigma, residual) |
|---|---:|---:|---:|
| tight (lowest third) | 475 | 6679 | -0.040 |
| middle | 960 | 6329 | -0.039 |
| loose (highest third) | 1841 | 4382 | -0.038 |

The same among the four highest forecasts of a field, where the quantile decides:

| field | results | corr(mean sigma, residual) | corr(mean sigma, 1/place) |
|---|---:|---:|---:|
| tight | 776 | +0.026 | +0.030 |
| middle | 776 | -0.077 | -0.130 |
| loose | 772 | -0.068 | -0.243 |

