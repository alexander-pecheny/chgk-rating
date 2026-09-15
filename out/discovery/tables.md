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

| features | tune NDCG | @3 | test NDCG | @3 | the columns |
|---:|---:|---:|---:|---:|---|
| 1 | 0.8973 | 0.7846 | 0.9082 | 0.8080 | `fld_gap_fourth` |
| 2 | 0.8955 | 0.7816 | 0.9048 | 0.8034 | `fld_gap_fourth`, `fld_z` |
| 3 | 0.8948 | 0.7785 | 0.9076 | 0.8077 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac` |
| 5 | 0.8976 | 0.7845 | 0.9056 | 0.8042 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac`, `fld_rank_q`, `fld_gap_first` |
| 8 | 0.8969 | 0.7830 | 0.9074 | 0.8054 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac`, `fld_rank_q`, `fld_gap_first`, `agg_phi_min`, `agg_thetaphi_sd`, `agg_phi_max` |
| 12 | 0.8962 | 0.7796 | 0.9079 | 0.8065 | `fld_gap_fourth`, `fld_z`, `fld_rank_frac`, `fld_rank_q`, `fld_gap_first`, `agg_phi_min`, `agg_thetaphi_sd`, `agg_phi_max`, `ed_team_resid`, `shp_q_minus_qbar`, `agg_vol_min`, `agg_games_min` |

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

CatBoost YetiRank: {'fold0': 0.8942993174588285, 'fold0_round3': 0.8938000554932253, 'test': 0.9076854882479792, 'test3': 0.8085178645214787, 'n_features': 127, 'trees': 138}
