# Round-3 mechanisms on the round-2 residual, 2018-09-01 to 2024-08-29

17392 results in 581 weighted in-person fields.

## Lever 2: does the package's difficulty change what a lopsided lineup is worth

Spread of theta against the residual, all fields: -0.003 over 17371 results.

| tercile of the field's difficulty | truedl | results | corr(spread of theta, residual) |
|---|---:|---:|---:|
| 1 | 4.13 | 5770 | +0.020 |
| 2 | 5.54 | 5770 | +0.001 |
| 3 | 6.54 | 5770 | +0.014 |

## Lever 4: the lineup's own uncertainty against the mean of its players'

| feature | weighted corr with the residual | results |
|---|---:|---:|
| mean sigma of the lineup | +0.006 | 17390 |
| sd(Q(L)), propagated through the tempered roster estimate | +0.001 | 17390 |

sd(Q(L)) is 0.39 of the mean sigma on average, and the two correlate at 0.932: they are close to the same number up to a scale.

## Lever 5: how much a point of forecast gap is worth in person

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

## Lever 6: series memory, the mean surprise over the last 5 in-person results

| feature | weighted corr with the residual | results |
|---|---:|---:|
| mean of the last 5 surprises | +0.032 | 13069 |
| the same, teams with 5+ in-person results behind them | +0.016 | 10655 |
| the same, teams with 10+ in-person results behind them | +0.015 | 8365 |
| the same, teams with 20+ in-person results behind them | +0.004 | 5755 |
| the same, the five highest forecasts of a field | +0.033 | 2559 |

## Lever 7: editor affinity, and whether it is a property of strong players

| cohort | weighted corr(mean past residual under these editors, residual) | results |
|---|---:|---:|
| every lineup with any history under the editors | +0.013 | 8834 |
| the weakest two fifths by mean theta | +0.003 | 3532 |
| the middle two fifths | +0.000 | 3532 |
| the strongest fifth | +0.055 | 1770 |
| the strongest twentieth | +0.098 | 441 |

50.8% of the results have any history under the tournament's editors at all.

## Lever 3: the record the rated window cannot see

| feature | weighted corr with the residual | results |
|---|---:|---:|
| median unrated tournaments per player, last 2 years | +0.011 | 17390 |
| the mean of the same | +0.011 | 17390 |

88.7% of the lineups have a player with at least one unrated tournament in the window; the mean count is 7.07.

