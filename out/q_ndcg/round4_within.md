# Round-4 diagnosis, 2018-09-01 to 2024-08-29

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

## Where each feature lives, and the best a bonus on it could buy

The mean ordering scores 0.8963 / @3 0.7834 on these fields.

| feature | share of variance inside a field | best bonus | NDCG | @3 |
|---|---:|---:|---:|---:|
| the team's own town is the tournament's | 0.57 | +100 | 0.8966 | 0.7819 |
| share of the lineup local to the town | 0.52 | +100 | 0.8965 | 0.7825 |
| in-person tournaments per player, last 7 days | 0.32 | +100 | 0.8963 | 0.7836 |
| the same, last 30 days | 0.48 | +0 | 0.8963 | 0.7834 |
| sd of the lineup's own strength | 0.60 | -2 | 0.8963 | 0.7834 |
