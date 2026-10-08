# /// script
# requires-python = ">=3.11"
# dependencies = ["lightgbm", "catboost", "pandas", "numpy", "shap"]
# ///
"""The boosted ranker of plan 8: how well could an in-person field be ordered?

`fit --features` writes one row per result of every in-person field, from the
state of that morning.  This script trains a gradient-boosted ranker on it,
scores it with **exactly** the NDCG of `tools/ndcg.py` -- relevance `1/place`,
the frozen weights, the same tie-break -- and reads the importances.  It is a
research instrument and ships nothing: what it produces is a ceiling for the
simple model, not a forecast.

Protocol.  Every tune-window number is **out of fold**: the 650 tune fields are
split into five, and each fifth is predicted by a model trained on everything
dated before the test window except that fifth -- which includes the 1043
in-person fields of 2013 to 2018, none of which either window scores.  The test
window is read once, by a model retrained on everything before it.  Early
stopping is on the weighted NDCG itself and not on LightGBM's own, because the
two disagree and ours is the one being optimised.

`--base` starts the ranker from the shipped forecast: within a field an affine
map of the forecast ranks exactly as the forecast does, so the ranker begins at
round 3 and every tree after that is a correction.

    uv run tools/rank_boost.py --stage all
    uv run tools/rank_boost.py --stage ceiling
"""

import json
import os
import sqlite3
import sys

import numpy as np
import pandas as pd

import lab

TUNE, TEST = lab.TUNE, lab.TEST
FOLDS = 5
BOOT = lab.BOOT
SEED = lab.SEED
OUT = "out/discovery"
OPTIONS = [
    ("--features", dict(default=f"{OUT}/features.db")),
    ("--stage", dict(default="all", help="all, report, or a comma list of " + "ceiling,importance,ablation,forward,linear,catboost")),
    ("--out", dict(default=f"{OUT}/rank.json")),
]
STAGES = ["ceiling", "importance", "ablation", "forward", "linear", "catboost"]
# Knuth's multiplicative hash, which deals the tune fields into folds.
FOLD_HASH = 2_654_435_761
# The importance sample, the forward curve's sizes and the linear model's width.
SHAP_SAMPLE = 20_000
FORWARD = (1, 2, 3, 5, 8, 12)
LINEAR_FEATURES = 8
# The pairwise logistic: ridge, pairs per field, Newton steps and tolerance.
L2 = 10.0
MAX_PAIRS = 400
NEWTON_STEPS = 60
NEWTON_TOL = 1e-9
CATBOOST = {
    "loss_function": "YetiRank", "iterations": 800, "learning_rate": 0.03,
    "depth": 6, "verbose": False, "random_seed": SEED, "thread_count": 8,
}
CATBOOST_STOP = 80
# Labels: place `p` of the top ten is label `11 - p`; outside it, 0.
LABELS = 11

FAMILIES = {
    "lineup strength": "lin_",
    "player aggregates": "agg_",
    "lineup shape": "shp_",
    "team state": "tm_",
    "editor": "ed_",
    "field context": "fld_",
    "calendar and place": "cal_",
    "external systems": "ext_",
}
# Not features: the identifiers, the target, the metric's weight, and what the
# shipped model said.
NOT_FEATURES = ("tournament_id", "team_id", "date", "y_", "w_", "m_")

PARAMS = {
    "objective": "lambdarank",
    # Our own weighted NDCG is the eval, through `feval`.
    "metric": "None",
    # The gain of a label is the metric's own relevance: label `l` is place
    # `11 - l`, worth `1 / place`, and everything outside the top ten is worth
    # nothing to NDCG@3 and almost nothing to NDCG.
    "label_gain": [0.0] + [1.0 / (LABELS - l) for l in range(1, LABELS)],
    "lambdarank_truncation_level": 30,
    "learning_rate": 0.02,
    "num_leaves": 31,
    "min_data_in_leaf": 20,
    "feature_fraction": 0.7,
    "bagging_fraction": 0.8,
    "bagging_freq": 1,
    "lambda_l2": 1.0,
    "verbosity": -1,
    "num_threads": 8,
    "seed": SEED,
}
ROUNDS = 800
STOP = 100
# The scale the shipped forecast enters `--base` on. Any positive number ranks a
# field the same way; this one puts the field's spread where lambdarank's
# sigmoid can still move it.
BASE_SCALE = 4.0


# --------------------------------------------------------------------------- #
# the table


def label_of(place):
    p = int(round(place))
    return max(0, LABELS - p) if p < LABELS else 0


EXTERNAL = (
    ("gg", "out/ext/gg.db", ("forecast", "mu_before")),
    ("fun", "out/ext/fun.db", ("forecast",)),
    ("quest", "out/ext/quest.db", ("forecast",)),
)


def external(name, db, cols):
    """One external system's columns, keyed by result."""
    c = lab.connect(db)
    e = pd.read_sql(f"select tournament_id, team_id, {', '.join(cols)} from team_events", c)
    c.close()
    e = e.rename(columns={k: f"ext_{name}_{k}" for k in cols})
    return e.drop_duplicates(subset=["tournament_id", "team_id"])


def load(path):
    """The feature table, with the external systems joined on and labelled."""
    con = lab.connect(path)
    df = pd.read_sql("select * from features", con)
    con.close()
    for name, db, cols in EXTERNAL:
        if os.path.exists(db):
            df = df.merge(external(name, db, cols), on=["tournament_id", "team_id"], how="left")
        else:
            print(f"# {db} missing, its columns are left out", file=sys.stderr)
    # The field the metric scores is the field the engine scored: a result with
    # neither a lineup nor a team state is in neither.
    df = df[(df.y_place > 0) & df.m_forecast.notna()].copy()
    df = df[df.groupby("tournament_id")["team_id"].transform("size") >= lab.MIN_FIELD].copy()
    df["label"] = df.y_place.map(label_of)
    return df.sort_values(["tournament_id"], kind="stable").reset_index(drop=True)


def features_of(df, with_ext=True):
    cols = [c for c in df.columns if not c.startswith(NOT_FEATURES) and c != "label"]
    return cols if with_ext else [c for c in cols if not c.startswith("ext_")]


def window(df, w):
    return df[(df.date >= w[0]) & (df.date < w[1])]


def folds_of(tids):
    return {int(t): int((t * FOLD_HASH) % FOLDS) for t in tids}


# --------------------------------------------------------------------------- #
# scoring, in the metric of tools/ndcg.py


def per_tournament(df, score):
    """`{tournament: (NDCG, NDCG@3)}` of an order by `score`, under the metric's
    own tie-break: a shuffle seeded from the tournament id, so the stored
    finishing order never leaks into the rank."""
    out = {}
    s = pd.Series(score, index=df.index)
    for tid, g in df.groupby("tournament_id", sort=False):
        v = s.loc[g.index]
        if len(g) >= lab.MIN_FIELD and not v.isna().any():
            places = dict(zip(g.team_id, g.y_place))
            strength = dict(zip(g.team_id, v))
            order = lab.ranked(strength, strength.get, lab.Rng(int(tid)))
            out[int(tid)] = (lab.ndcg(order, places), lab.ndcg(order, places, lab.TOP))
    return out


def at(runs, k):
    return {t: v[k] for t, v in runs.items()}


def wmean(runs, w, tids=None, k=0):
    tids = list(runs) if tids is None else [t for t in tids if t in runs]
    return lab.wmean(at(runs, k), tids, w or None)


def paired(a, b, w, k=0, boot=BOOT):
    """Weighted difference, its bootstrap SE, and how many fields both cover."""
    keys = sorted(set(a) & set(b))
    if not keys:
        return lab.NAN, lab.NAN, 0
    return (*lab.paired(at(a, k), at(b, k), keys, w, boot), len(keys))


# --------------------------------------------------------------------------- #
# the ranker


def init_score(g):
    """The shipped forecast as a starting score, standardised within its field."""
    z = g.groupby("tournament_id").m_forecast.transform(
        lambda x: (x - x.mean()) / (x.std() if x.std() and x.std() > 0 else 1.0)
    )
    return (BASE_SCALE * z.fillna(0.0)).to_numpy(dtype=np.float64)


def dataset(lgb, g, cols, weights, base):
    d = lgb.Dataset(
        g[cols].to_numpy(dtype=np.float32),
        label=g.label.to_numpy(),
        group=g.groupby("tournament_id", sort=False).size().to_numpy(),
        weight=g.tournament_id.map(lambda t: weights.get(int(t), 0.0)).to_numpy(),
        feature_name=list(cols),
        free_raw_data=False,
    )
    if base:
        d.set_init_score(init_score(g))
    return d


def shuffled(n, tid):
    """`0..n-1` in the metric's tie-break order for tournament `tid`."""
    o = list(range(n))
    lab.Rng(tid).shuffle(o)
    return o


def field_layout(g, weights):
    """Per field of `g`: its row bounds, weight, ideal DCG, tie-break permutation
    and discounts."""
    place = g.y_place.to_numpy(dtype=np.float64)
    bounds, wts, ideal, perm, disc = [], [], [], [], []
    start = 0
    for t, n in zip(*np.unique(g.tournament_id.to_numpy(), return_counts=True)):
        d = 1.0 / np.log2(np.arange(n) + 2)
        bounds.append((start, start + n))
        wts.append(weights.get(int(t), 0.0))
        ideal.append((np.sort(1.0 / place[start : start + n])[::-1] * d).sum())
        disc.append(d)
        perm.append(np.asarray(shuffled(n, int(t))))
        start += n
    return place, bounds, np.asarray(wts), np.asarray(ideal), perm, disc


def make_feval(g, weights):
    """The metric of `tools/ndcg.py`, vectorised, as a LightGBM eval.

    The tie-break matters more here than anywhere else. An early ensemble puts
    whole blocks of a field on the same score, and the rows are stored in
    finishing order, so a stable sort would read the answer off the table: with
    one tree it scores 0.935 where the metric says 0.866, and early stopping then
    picks that tree and stops. Every group is therefore pre-shuffled by
    `lab.Rng` seeded from its tournament id, exactly as the metric does.
    """
    place, bounds, wts, ideal, perm, disc = field_layout(g, weights)

    def feval(preds, _ds):
        num = 0.0
        for k, (a, b) in enumerate(bounds):
            o = perm[k][np.argsort(-preds[a:b][perm[k]], kind="stable")]
            num += wts[k] * (1.0 / place[a:b][o] * disc[k]).sum() / ideal[k]
        return "wndcg", num / wts.sum(), True

    return feval


def predict(m, g, cols, base, it=None):
    p = m.predict(g[cols].to_numpy(dtype=np.float32), num_iteration=it)
    return pd.Series(p + (init_score(g) if base else 0.0), index=g.index)


def fit_one(lgb, trn, va, cols, weights, base, params=None):
    p = dict(PARAMS if params is None else params)
    m = lgb.train(
        p,
        dataset(lgb, trn, cols, weights, base),
        num_boost_round=ROUNDS,
        valid_sets=[dataset(lgb, va, cols, weights, base)],
        feval=make_feval(va, weights),
        callbacks=[lgb.early_stopping(STOP, verbose=False)],
    )
    return m, m.best_iteration or 1


def pool(df, drop):
    """Everything dated before the test window, less the held-out fields."""
    g = df[df.date < TEST[0]]
    return g[~g.tournament_id.isin(drop)] if drop else g


def tune_folds(df):
    """The tune window and each row's fold."""
    tune = window(df, TUNE)
    return tune, tune.tournament_id.map(folds_of(tune.tournament_id.unique()))


def cv(lgb, df, cols, weights, base=False, params=None):
    """Out-of-fold predictions over the tune window, and the fold models."""
    tune, fold = tune_folds(df)
    oof = pd.Series(np.nan, index=tune.index)
    models, iters, vas = [], [], []
    for k in range(FOLDS):
        va = tune[fold == k]
        m, it = fit_one(lgb, pool(df, set(va.tournament_id)), va, cols, weights, base, params)
        oof.loc[va.index] = predict(m, va, cols, base, it)
        models.append(m)
        iters.append(it)
        vas.append(va)
    return oof, models, vas, int(round(sum(iters) / len(iters)))


def test_model(lgb, df, cols, weights, base, rounds, params=None):
    p = dict(PARAMS if params is None else params)
    return lgb.train(p, dataset(lgb, pool(df, None), cols, weights, base), num_boost_round=rounds)


def line(tag, runs, w):
    return (
        f"{tag:22s} w {wmean(runs, w):.4f} @3 {wmean(runs, w, k=1):.4f} "
        f"u {wmean(runs, None):.4f} @3 {wmean(runs, None, k=1):.4f} n {len(runs)}"
    )


def means(r, weights):
    """Weighted NDCG and NDCG@3."""
    return wmean(r, weights), wmean(r, weights, k=1)


# --------------------------------------------------------------------------- #
# stages

CEILINGS = (
    ("ranker", False, False),
    ("ranker_ext", True, False),
    ("ranker_base", False, True),
    ("ranker_base_ext", True, True),
)


def ceiling_of(lgb, df, weights, ext, base):
    """One ranker: out-of-fold tune scores and a retrained test score."""
    tune, test = window(df, TUNE), window(df, TEST)
    cols = features_of(df, ext)
    oof, _, _, it = cv(lgb, df, cols, weights, base)
    m = test_model(lgb, df, cols, weights, base, it)
    return {
        "tune": per_tournament(tune, oof),
        "test": per_tournament(test, predict(m, test, cols, base)),
        "n_features": len(cols),
        "iters": it,
    }


def stage_ceiling(lgb, df, weights, report):
    tune, test = window(df, TUNE), window(df, TEST)
    out = {"round3": {"tune": per_tournament(tune, tune.m_forecast),
                      "test": per_tournament(test, test.m_forecast)}}
    for tag, ext, base in CEILINGS:
        out[tag] = ceiling_of(lgb, df, weights, ext, base)
        print(f"# {tag}: {out[tag]['n_features']} features, {out[tag]['iters']} trees", file=sys.stderr)
    out.update(external_ceilings(df))
    for tag, r in out.items():
        for wnd in ("tune", "test"):
            print(line(f"{tag} {wnd}", r[wnd], weights), file=sys.stderr)
    report["ceiling"] = out
    return out


def external_ceilings(df):
    """The other systems' forecasts, scored over the fields they cover."""
    tune, test, out = window(df, TUNE), window(df, TEST), {}
    for name in ("fun", "quest"):
        col = f"ext_{name}_forecast"
        if col in df.columns:
            a, b = tune.dropna(subset=[col]), test.dropna(subset=[col])
            out[name] = {"tune": per_tournament(a, a[col]), "test": per_tournament(b, b[col])}
    return out


def permute_within(Xk, groups, j, rng):
    """Column `j` of `Xk` shuffled within each field."""
    start = 0
    for _, n in zip(*np.unique(groups, return_counts=True)):
        Xk[start : start + n, j] = rng.permutation(Xk[start : start + n, j])
        start += n
    return Xk


def permutation_importance(models, vas, cols, tune, weights, base):
    """The weighted NDCG lost when each column is shuffled within its fields."""
    rng = np.random.default_rng(SEED)
    X = [v[cols].to_numpy(dtype=np.float32) for v in vas]
    G = [v.tournament_id.to_numpy() for v in vas]
    perm = {}
    for j, name in enumerate(cols):
        pred = pd.Series(np.nan, index=tune.index)
        for k, v in enumerate(vas):
            pred.loc[v.index] = models[k].predict(permute_within(X[k].copy(), G[k], j, rng))
        perm[name] = base - wmean(per_tournament(tune, pred), weights)
    return perm


def mean_shap(model, tune, cols):
    """Mean |SHAP| per column over a sample of the tune window."""
    sample = tune.sample(n=min(SHAP_SAMPLE, len(tune)), random_state=SEED)
    c = model.predict(sample[cols].to_numpy(dtype=np.float32), pred_contrib=True)
    return np.abs(c[:, : len(cols)]).mean(axis=0)


def stage_importance(lgb, df, weights, report):
    """Gain, permutation (within a field, out of fold) and SHAP."""
    tune = window(df, TUNE)
    cols = features_of(df, with_ext=True)
    oof, models, vas, _ = cv(lgb, df, cols, weights)
    base = wmean(per_tournament(tune, oof), weights)
    gain = sum(m.feature_importance("gain") for m in models)
    shap = mean_shap(models[0], tune, cols)
    perm = permutation_importance(models, vas, cols, tune, weights, base)
    report["importance"] = {"base": base, "gain": dict(zip(cols, [float(x) for x in gain])),
                            "shap": dict(zip(cols, [float(x) for x in shap])), "perm": perm}
    report["perm_order"] = sorted(perm, key=lambda c: -perm[c])
    print("# importance done", file=sys.stderr)
    return report["importance"]


def stage_ablation(lgb, df, weights, report):
    tune = window(df, TUNE)
    all_cols = features_of(df, with_ext=True)
    oof, _, _, _ = cv(lgb, df, all_cols, weights)
    rows = {"all": [*means(per_tournament(tune, oof), weights), len(all_cols)]}
    for name, pre in FAMILIES.items():
        cols = [c for c in all_cols if not c.startswith(pre)]
        if len(cols) < len(all_cols):
            o, _, _, _ = cv(lgb, df, cols, weights)
            rows[f"minus {name}"] = [*means(per_tournament(tune, o), weights), len(cols)]
            print(f"# ablation minus {name}: {rows[f'minus {name}'][0]:.4f}", file=sys.stderr)
    report["ablation"] = rows
    return rows


def forward_step(lgb, df, weights, cols):
    tune, test = window(df, TUNE), window(df, TEST)
    oof, _, _, it = cv(lgb, df, cols, weights)
    (t, t3) = means(per_tournament(tune, oof), weights)
    m = test_model(lgb, df, cols, weights, False, it)
    (s, s3) = means(per_tournament(test, predict(m, test, cols, False)), weights)
    return {"tune": t, "tune3": t3, "test": s, "test3": s3, "cols": cols}


def stage_forward(lgb, df, weights, report, order):
    rows = {}
    for k in FORWARD:
        rows[k] = forward_step(lgb, df, weights, order[:k])
        print(f"# forward {k}: tune {rows[k]['tune']:.4f} test {rows[k]['test']:.4f}", file=sys.stderr)
    report["forward"] = rows
    return rows


# --------------------------------------------------------------------------- #
# the linear ranker, the first distillation


def standardise(df, cols):
    tune = window(df, TUNE)
    med = tune[cols].median()
    mu = tune[cols].fillna(med).mean()
    sd = tune[cols].fillna(med).std().replace(0, 1.0)
    return med, mu, sd


def zed(g, cols, med, mu, sd):
    return ((g[cols].fillna(med) - mu) / sd).to_numpy(dtype=np.float64)


def pair_rows(tune, Z, weights):
    """Feature differences of every ordered pair of every weighted tune field,
    each carrying its field's weight."""
    rng = np.random.default_rng(SEED)
    place = tune.y_place.to_numpy()
    rows, wts, start = [], [], 0
    for t, n in zip(*np.unique(tune.tournament_id.to_numpy(), return_counts=True)):
        w, sl = weights.get(int(t), 0.0), np.arange(start, start + n)
        start += n
        pairs = [(a, b) for a in sl for b in sl if place[a] < place[b]] if w > 0 and n >= 2 else []
        # A field of a hundred is five thousand pairs; the cap keeps one open
        # championship from outvoting fifty ordinary fields.
        if len(pairs) > MAX_PAIRS:
            pairs = [pairs[i] for i in rng.choice(len(pairs), MAX_PAIRS, replace=False)]
        rows += [Z[a] - Z[b] for a, b in pairs]
        wts += [w] * len(pairs)
    return np.asarray(rows), np.asarray(wts)


def newton(X, wv, l2):
    """Ridge-penalised logistic regression without intercept, by Newton steps."""
    beta = np.zeros(X.shape[1])
    for _ in range(NEWTON_STEPS):
        p = 1.0 / (1.0 + np.exp(-(X @ beta)))
        g = X.T @ (wv * (1.0 - p)) - l2 * beta
        H = (X * (wv * p * (1.0 - p))[:, None]).T @ X + l2 * np.eye(X.shape[1])
        step = np.linalg.solve(H, g)
        beta += step
        if np.abs(step).max() < NEWTON_TOL:
            break
    return beta


def linear_rank(df, cols, weights, l2=L2):
    """Pairwise logistic on standardised features: `P(i above j) = s(b . (x_i - x_j))`,
    over the pairs of every tune field, each pair carrying its field's weight."""
    norm = standardise(df, cols)
    X, wv = pair_rows(window(df, TUNE), zed(window(df, TUNE), cols, *norm), weights)
    return newton(X, wv / wv.mean(), l2), norm, len(X)


def stage_linear(df, weights, report, cols):
    beta, norm, npairs = linear_rank(df, cols, weights)
    out = {"cols": list(cols), "beta": [float(b) for b in beta], "pairs": npairs}
    for tag, wnd in (("tune", TUNE), ("test", TEST)):
        g = window(df, wnd)
        r = per_tournament(g, pd.Series(zed(g, cols, *norm) @ beta, index=g.index))
        out[tag], out[tag + "3"] = means(r, weights)
        out[tag + "_runs"] = r
        print(line(f"linear {tag}", r, weights), file=sys.stderr)
    report["linear"] = out
    return out


def catboost_pool(Pool, g, cols, weights):
    return Pool(
        g[cols].to_numpy(dtype=np.float32),
        label=g.label.to_numpy(dtype=np.float32),
        group_id=g.tournament_id.to_numpy(),
        weight=g.tournament_id.map(lambda t: weights.get(int(t), 0.0)).to_numpy(),
    )


def stage_catboost(df, weights, report):
    try:
        from catboost import CatBoost, Pool
    except (ImportError, OSError) as e:  # pragma: no cover
        report["catboost"] = {"error": str(e)}
        print(f"# catboost unavailable: {e}", file=sys.stderr)
        return None
    return catboost_fold0(df, weights, report, CatBoost, Pool)


def catboost_fold0(df, weights, report, CatBoost, Pool):
    """YetiRank trained without fold 0 of the tune window, scored on it and on test."""
    tune, fold = tune_folds(df)
    test, cols = window(df, TEST), features_of(df, with_ext=False)
    va = tune[fold == 0]
    m = CatBoost(CATBOOST)
    m.fit(catboost_pool(Pool, pool(df, set(va.tournament_id)), cols, weights),
          eval_set=catboost_pool(Pool, va, cols, weights), early_stopping_rounds=CATBOOST_STOP)
    rv, rt = (per_tournament(g, pd.Series(m.predict(catboost_pool(Pool, g, cols, weights)), index=g.index))
              for g in (va, test))
    return catboost_report(report, m, rv, rt, per_tournament(va, va.m_forecast), weights, len(cols))


def catboost_report(report, m, rv, rt, b, weights, n_features):
    report["catboost"] = {
        "fold0": wmean(rv, weights), "fold0_round3": wmean(b, weights),
        "test": wmean(rt, weights), "test3": wmean(rt, weights, k=1),
        "n_features": n_features, "trees": int(m.tree_count_),
    }
    print(f"# catboost fold0 {wmean(rv, weights):.4f} against round 3 "
          f"{wmean(b, weights):.4f}, test {wmean(rt, weights):.4f}", file=sys.stderr)
    return report["catboost"]


# --------------------------------------------------------------------------- #
# the tables of out/discovery/report.md


def runs_of(d):
    return {int(k): v for k, v in d.items()}


NAMES = {
    "round3": "round 3, the shipped forecast",
    "ranker": "the ranker, no external features",
    "ranker_ext": "the ranker, with them",
    "ranker_base": "the ranker started from round 3",
    "ranker_base_ext": "the same, with the external features",
    "fun": "rating.chgk.fun",
    "quest": "chgk.quest",
}
RANKERS = ("ranker", "ranker_ext", "ranker_base", "ranker_base_ext")
REFERENCES = ("round3", "fun", "quest")
WINDOWS = ("tune", "test")
TOP_FEATURES = 25


def four_means(runs, weights):
    """Weighted and unweighted NDCG and NDCG@3 as table cells."""
    return " | ".join(f"{wmean(runs, w, k=k):.4f}" for w in (weights, None) for k in (0, 1))


def report_ceiling(c, weights):
    print("| system | tune | @3 | tune, unweighted | @3 | test | @3 | test, unweighted | @3 | fields |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for k, label in NAMES.items():
        if k in c:
            a, b = runs_of(c[k]["tune"]), runs_of(c[k]["test"])
            print(f"| {label} | {four_means(a, weights)} | {four_means(b, weights)} | {len(a)} / {len(b)} |")
    print("\nPaired bootstrap over the fields both rows cover, 2000 resamples, weighted.\n")
    print("| a | b | window | NDCG | SE | NDCG@3 | SE | fields |")
    print("|---|---|---|---:|---:|---:|---:|---:|")
    pairs = [(a, b, wnd) for a in RANKERS if a in c for b in REFERENCES if b in c for wnd in WINDOWS]
    for a, b, wnd in pairs:
        (d, se, n), (d3, se3, _) = (paired(runs_of(c[a][wnd]), runs_of(c[b][wnd]), weights, k=k) for k in (0, 1))
        print(f"| {NAMES[a]} | {NAMES[b]} | {wnd} | {d:+.4f} | {se:.4f} | {d3:+.4f} | {se3:.4f} | {n} |")


def report_importance(r, imp):
    gr = {c: i + 1 for i, c in enumerate(sorted(imp["gain"], key=lambda x: -imp["gain"][x]))}
    sr = {c: i + 1 for i, c in enumerate(sorted(imp["shap"], key=lambda x: -imp["shap"][x]))}
    print(f"\nOut-of-fold weighted NDCG of the model the importances are read off: {imp['base']:.4f}.\n")
    print("| # | feature | family | permutation | gain rank | SHAP rank |")
    print("|---:|---|---|---:|---:|---:|")
    for i, c in enumerate(r["perm_order"][:TOP_FEATURES]):
        fam = next((k for k, v in FAMILIES.items() if c.startswith(v)), "")
        print(f"| {i + 1} | `{c}` | {fam} | {imp['perm'][c]:+.5f} | {gr[c]} | {sr[c]} |")


def report_ablation(ab):
    print("\n| features | columns | tune NDCG | @3 | cost |")
    print("|---|---:|---:|---:|---:|")
    for k, v in ab.items():
        cost = "-" if k == "all" else f"{v[0] - ab['all'][0]:+.4f}"
        print(f"| {k} | {v[2]} | {v[0]:.4f} | {v[1]:.4f} | {cost} |")


def report_forward(fw):
    print("\n| features | tune NDCG | @3 | test NDCG | @3 | the columns |")
    print("|---:|---:|---:|---:|---:|---|")
    for k in sorted(fw, key=int):
        v = fw[k]
        print(f"| {k} | {v['tune']:.4f} | {v['tune3']:.4f} | {v['test']:.4f} "
              f"| {v['test3']:.4f} | {', '.join('`' + x + '`' for x in v['cols'])} |")


def report_linear(lin, c, weights):
    print(f"\nPairwise logistic on {lin['pairs']} pairs of the tune window, features standardised.\n")
    print("| feature | coefficient |")
    print("|---|---:|")
    for col, b in zip(lin["cols"], lin["beta"]):
        print(f"| `{col}` | {b:+.3f} |")
    print(f"\ntune {lin['tune']:.4f} @3 {lin['tune3']:.4f}, test {lin['test']:.4f} @3 {lin['test3']:.4f}")
    for b_k, wnd in [(b, wnd) for b in ("round3", "fun") if b in c for wnd in WINDOWS]:
        d, se, n = paired(runs_of(lin[wnd + "_runs"]), runs_of(c[b_k][wnd]), weights)
        print(f"  linear minus {NAMES[b_k]}, {wnd}: {d:+.4f} se {se:.4f} ({n})")


SECTIONS = [
    ("ceiling", lambda r, c, w: report_ceiling(c, w)),
    ("importance", lambda r, c, w: report_importance(r, r["importance"])),
    ("ablation", lambda r, c, w: report_ablation(r["ablation"])),
    ("forward", lambda r, c, w: report_forward(r["forward"])),
    ("linear", lambda r, c, w: report_linear(r["linear"], c, w)),
    ("catboost", lambda r, c, w: print(f"\nCatBoost YetiRank: {r['catboost']}")),
]


def stage_report(path, weights):
    """Every table of the report, from a finished `rank.json`."""
    with open(path, encoding="utf-8") as f:
        r = json.load(f)
    for key, show in SECTIONS:
        if r.get(key):
            show(r, r.get("ceiling", {}), weights)


def plain(o):
    """`o` with numpy scalars and tuples turned into what `json` writes."""
    if isinstance(o, dict):
        return {str(k): plain(v) for k, v in o.items()}
    if isinstance(o, (list, tuple)):
        return [plain(x) for x in o]
    if isinstance(o, (np.floating, np.integer)):
        return o.item()
    return o


MODELS = (("ceiling", stage_ceiling), ("importance", stage_importance), ("ablation", stage_ablation))


def run_stages(lgb, df, weights, stages):
    report = {}
    for name, stage in MODELS:
        if name in stages:
            stage(lgb, df, weights, report)
    distil(lgb, df, weights, stages, report)
    if "catboost" in stages:
        stage_catboost(df, weights, report)
    return report


def distil(lgb, df, weights, stages, report):
    """The ranker may lean on an external column; the distillation may not, so
    the forward curve and the linear model are over what could ship."""
    order = report.get("perm_order")
    own = [c for c in order or [] if not c.startswith("ext_")]
    if order and "forward" in stages:
        stage_forward(lgb, df, weights, report, own)
    if order and "linear" in stages:
        stage_linear(df, weights, report, own[:LINEAR_FEATURES])


def main():
    a = lab.cli(__doc__, OPTIONS)
    weights = lab.load_weights()
    if a.stage == "report":
        stage_report(a.out, weights)
        return
    import lightgbm as lgb

    df = load(a.features)
    print(f"# {len(df)} results in {df.tournament_id.nunique()} fields; {len(features_of(df, False))} features, "
          f"{len(features_of(df, True))} with the external ones", file=sys.stderr)
    os.makedirs(OUT, exist_ok=True)
    write(a.out, run_stages(lgb, df, weights, STAGES if a.stage == "all" else a.stage.split(",")))


def write(path, report):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(plain(report), f)
    print(f"# wrote {path}", file=sys.stderr)


if __name__ == "__main__":
    main()
