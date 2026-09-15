# The leakage check

Columns: 138 in all, 128 of them features.
No feature correlates with the finishing place above 0.90 within a field; the best of them is the model's own forecast, which is not a feature.

## Re-derived from strictly earlier rows

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

## The scramble test

Scrambled the first in-person field on or after 2024-01-01: tournament 10087 of 2024-01-06, 19 results, with 10518 results in the 391 fields that follow it.
Rows of the fields before it that moved: 0.
**No feature of the 19 results moved** when that field's own places and masks were permuted: not one of the 128 columns.
Columns that moved in the fields after it, as they must: 77 of 128.

**PASSED**: census ok, re-derivation ok, scramble ok.
