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

import argparse
import json
import os
import random
import sqlite3
import sys
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ndcg as N  # noqa: E402

TUNE = ("2018-09-01", "2024-08-29")
TEST = ("2024-08-30", "2026-08-27")
FOLDS = 5
BOOT = 2000
SEED = 20260914
OUT = "out/discovery"

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
    "label_gain": [0.0] + [1.0 / (11 - l) for l in range(1, 11)],
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
    return max(0, 11 - p) if p <= 10 else 0


def load(path):
    """The feature table, with the external systems joined on and labelled."""
    con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    df = pd.read_sql("select * from features", con)
    con.close()
    for name, db, cols in (
        ("gg", "out/ext/gg.db", ("forecast", "mu_before")),
        ("fun", "out/ext/fun.db", ("forecast",)),
        ("quest", "out/ext/quest.db", ("forecast",)),
    ):
        if not os.path.exists(db):
            print(f"# {db} missing, its columns are left out", file=sys.stderr)
            continue
        c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        sel = ", ".join(cols)
        e = pd.read_sql(f"select tournament_id, team_id, {sel} from team_events", c)
        c.close()
        e = e.rename(columns={k: f"ext_{name}_{k}" for k in cols})
        e = e.drop_duplicates(subset=["tournament_id", "team_id"])
        df = df.merge(e, on=["tournament_id", "team_id"], how="left")
    # The field the metric scores is the field the engine scored: a result with
    # neither a lineup nor a team state is in neither.
    df = df[(df.y_place > 0) & df.m_forecast.notna()].copy()
    n = df.groupby("tournament_id")["team_id"].transform("size")
    df = df[n >= 5].copy()
    df["label"] = df.y_place.map(label_of)
    return df.sort_values(["tournament_id"], kind="stable").reset_index(drop=True)


def features_of(df, with_ext=True):
    cols = [c for c in df.columns if not c.startswith(NOT_FEATURES) and c != "label"]
    return cols if with_ext else [c for c in cols if not c.startswith("ext_")]


def window(df, w):
    return df[(df.date >= w[0]) & (df.date < w[1])]


def folds_of(tids):
    return {int(t): int((t * 2_654_435_761) % FOLDS) for t in tids}


# --------------------------------------------------------------------------- #
# scoring, in the metric of tools/ndcg.py


def per_tournament(df, score):
    """`{tournament: (NDCG, NDCG@3)}` of an order by `score`, under the metric's
    own tie-break: a shuffle seeded from the tournament id, so the stored
    finishing order never leaks into the rank."""
    out = {}
    s = pd.Series(score, index=df.index)
    for tid, g in df.groupby("tournament_id", sort=False):
        if len(g) < 5:
            continue
        v = s.loc[g.index]
        if v.isna().any():
            continue
        places = dict(zip(g.team_id, g.y_place))
        strength = dict(zip(g.team_id, v))
        order = list(strength)
        N.Rng(int(tid)).shuffle(order)
        order.sort(key=lambda t: -strength[t])
        out[int(tid)] = (
            N.ndcg(order, places, None, strength),
            N.ndcg(order, places, 3, strength),
        )
    return out


def wmean(runs, w, tids=None, k=0):
    tids = list(runs) if tids is None else [t for t in tids if t in runs]
    num = sum((w.get(t, 0.0) if w else 1.0) * runs[t][k] for t in tids)
    den = sum((w.get(t, 0.0) if w else 1.0) for t in tids)
    return num / den if den > 0 else float("nan")


def paired(a, b, w, k=0, boot=BOOT):
    keys = sorted(set(a) & set(b))
    if not keys:
        return float("nan"), float("nan"), 0
    rng = random.Random(SEED)

    def diff(sample):
        num = sum(w.get(t, 0.0) * (a[t][k] - b[t][k]) for t in sample)
        den = sum(w.get(t, 0.0) for t in sample)
        return num / den if den > 0 else float("nan")

    point = diff(keys)
    draws = [diff([keys[rng.randrange(len(keys))] for _ in keys]) for _ in range(boot)]
    m = sum(draws) / len(draws)
    se = (sum((d - m) ** 2 for d in draws) / len(draws)) ** 0.5
    return point, se, len(keys)


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


def make_feval(g, weights):
    """The metric of `tools/ndcg.py`, vectorised, as a LightGBM eval.

    The tie-break matters more here than anywhere else. An early ensemble puts
    whole blocks of a field on the same score, and the rows are stored in
    finishing order, so a stable sort would read the answer off the table: with
    one tree it scores 0.935 where the metric says 0.866, and early stopping then
    picks that tree and stops. Every group is therefore pre-shuffled by
    `ndcg.Rng` seeded from its tournament id, exactly as the metric does.
    """
    tid = g.tournament_id.to_numpy()
    place = g.y_place.to_numpy(dtype=np.float64)
    bounds, wts, ideal, perm, disc = [], [], [], [], []
    start = 0
    for t, n in zip(*np.unique(tid, return_counts=True)):
        bounds.append((start, start + n))
        wts.append(weights.get(int(t), 0.0))
        rel = np.sort(1.0 / place[start : start + n])[::-1]
        d = 1.0 / np.log2(np.arange(n) + 2)
        ideal.append((rel * d).sum())
        disc.append(d)
        o = list(range(n))
        N.Rng(int(t)).shuffle(o)
        perm.append(np.asarray(o))
        start += n
    wts, ideal = np.asarray(wts), np.asarray(ideal)

    def feval(preds, _ds):
        num = 0.0
        for k, (a, b) in enumerate(bounds):
            p = perm[k]
            o = p[np.argsort(-preds[a:b][p], kind="stable")]
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


def cv(lgb, df, cols, weights, base=False, params=None):
    """Out-of-fold predictions over the tune window, and the fold models."""
    tune = window(df, TUNE)
    fold = tune.tournament_id.map(folds_of(tune.tournament_id.unique()))
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


# --------------------------------------------------------------------------- #
# stages


def stage_ceiling(lgb, df, weights, report):
    tune, test = window(df, TUNE), window(df, TEST)
    out = {"round3": {"tune": per_tournament(tune, tune.m_forecast),
                      "test": per_tournament(test, test.m_forecast)}}
    for tag, ext, base in (
        ("ranker", False, False),
        ("ranker_ext", True, False),
        ("ranker_base", False, True),
        ("ranker_base_ext", True, True),
    ):
        cols = features_of(df, ext)
        oof, _, _, it = cv(lgb, df, cols, weights, base)
        m = test_model(lgb, df, cols, weights, base, it)
        out[tag] = {
            "tune": per_tournament(tune, oof),
            "test": per_tournament(test, predict(m, test, cols, base)),
            "n_features": len(cols),
            "iters": it,
        }
        print(f"# {tag}: {len(cols)} features, {it} trees", file=sys.stderr)
    for name in ("fun", "quest"):
        col = f"ext_{name}_forecast"
        if col in df.columns:
            a, b = tune.dropna(subset=[col]), test.dropna(subset=[col])
            out[name] = {"tune": per_tournament(a, a[col]), "test": per_tournament(b, b[col])}
    for tag, r in out.items():
        for wnd in ("tune", "test"):
            print(line(f"{tag} {wnd}", r[wnd], weights), file=sys.stderr)
    report["ceiling"] = out
    return out


def stage_importance(lgb, df, weights, report):
    """Gain, permutation (within a field, out of fold) and SHAP."""
    tune = window(df, TUNE)
    cols = features_of(df, with_ext=True)
    oof, models, vas, _ = cv(lgb, df, cols, weights)
    base = wmean(per_tournament(tune, oof), weights)
    gain = sum(m.feature_importance("gain") for m in models)
    sample = tune.sample(n=min(20_000, len(tune)), random_state=SEED)
    c = models[0].predict(sample[cols].to_numpy(dtype=np.float32), pred_contrib=True)
    shap = np.abs(c[:, : len(cols)]).mean(axis=0)
    rng = np.random.default_rng(SEED)
    X = [v[cols].to_numpy(dtype=np.float32) for v in vas]
    G = [v.tournament_id.to_numpy() for v in vas]
    perm = {}
    for j, name in enumerate(cols):
        pred = pd.Series(np.nan, index=tune.index)
        for k, v in enumerate(vas):
            Xk = X[k].copy()
            start = 0
            for _, n in zip(*np.unique(G[k], return_counts=True)):
                Xk[start : start + n, j] = rng.permutation(Xk[start : start + n, j])
                start += n
            pred.loc[v.index] = models[k].predict(Xk)
        perm[name] = base - wmean(per_tournament(tune, pred), weights)
    report["importance"] = {
        "base": base,
        "gain": dict(zip(cols, [float(x) for x in gain])),
        "shap": dict(zip(cols, [float(x) for x in shap])),
        "perm": perm,
    }
    report["perm_order"] = sorted(perm, key=lambda c: -perm[c])
    print("# importance done", file=sys.stderr)
    return report["importance"]


def stage_ablation(lgb, df, weights, report):
    tune = window(df, TUNE)
    all_cols = features_of(df, with_ext=True)
    oof, _, _, _ = cv(lgb, df, all_cols, weights)
    r = per_tournament(tune, oof)
    rows = {"all": [wmean(r, weights), wmean(r, weights, k=1), len(all_cols)]}
    for name, pre in FAMILIES.items():
        cols = [c for c in all_cols if not c.startswith(pre)]
        if len(cols) == len(all_cols):
            continue
        o, _, _, _ = cv(lgb, df, cols, weights)
        r = per_tournament(tune, o)
        rows[f"minus {name}"] = [wmean(r, weights), wmean(r, weights, k=1), len(cols)]
        print(f"# ablation minus {name}: {rows[f'minus {name}'][0]:.4f}", file=sys.stderr)
    report["ablation"] = rows
    return rows


def stage_forward(lgb, df, weights, report, order):
    tune, test = window(df, TUNE), window(df, TEST)
    rows = {}
    for k in (1, 2, 3, 5, 8, 12):
        cols = order[:k]
        oof, _, _, it = cv(lgb, df, cols, weights)
        r = per_tournament(tune, oof)
        m = test_model(lgb, df, cols, weights, False, it)
        rt = per_tournament(test, predict(m, test, cols, False))
        rows[k] = {
            "tune": wmean(r, weights), "tune3": wmean(r, weights, k=1),
            "test": wmean(rt, weights), "test3": wmean(rt, weights, k=1),
            "cols": cols,
        }
        print(f"# forward {k}: tune {rows[k]['tune']:.4f} test {rows[k]['test']:.4f}",
              file=sys.stderr)
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


def linear_rank(df, cols, weights, l2=10.0):
    """Pairwise logistic on standardised features: `P(i above j) = s(b . (x_i - x_j))`,
    over the pairs of every tune field, each pair carrying its field's weight."""
    tune = window(df, TUNE)
    med, mu, sd = standardise(df, cols)
    Z = zed(tune, cols, med, mu, sd)
    rng = np.random.default_rng(SEED)
    tid = tune.tournament_id.to_numpy()
    place = tune.y_place.to_numpy()
    rows, wts, start = [], [], 0
    for t, n in zip(*np.unique(tid, return_counts=True)):
        w = weights.get(int(t), 0.0)
        sl = np.arange(start, start + n)
        start += n
        if w <= 0 or n < 2:
            continue
        pairs = [(a, b) for a in sl for b in sl if place[a] < place[b]]
        # A field of a hundred is five thousand pairs; the cap keeps one open
        # championship from outvoting fifty ordinary fields.
        if len(pairs) > 400:
            pairs = [pairs[i] for i in rng.choice(len(pairs), 400, replace=False)]
        for a, b in pairs:
            rows.append(Z[a] - Z[b])
            wts.append(w)
    X = np.asarray(rows)
    wv = np.asarray(wts)
    wv = wv / wv.mean()
    beta = np.zeros(X.shape[1])
    for _ in range(60):
        p = 1.0 / (1.0 + np.exp(-(X @ beta)))
        g = X.T @ (wv * (1.0 - p)) - l2 * beta
        s = wv * p * (1.0 - p)
        H = (X * s[:, None]).T @ X + l2 * np.eye(X.shape[1])
        step = np.linalg.solve(H, g)
        beta += step
        if np.abs(step).max() < 1e-9:
            break
    return beta, (med, mu, sd), len(X)


def stage_linear(df, weights, report, cols):
    beta, norm, npairs = linear_rank(df, cols, weights)
    out = {"cols": list(cols), "beta": [float(b) for b in beta], "pairs": npairs}
    for tag, wnd in (("tune", TUNE), ("test", TEST)):
        g = window(df, wnd)
        r = per_tournament(g, pd.Series(zed(g, cols, *norm) @ beta, index=g.index))
        out[tag] = wmean(r, weights)
        out[tag + "3"] = wmean(r, weights, k=1)
        out[tag + "_runs"] = r
        print(line(f"linear {tag}", r, weights), file=sys.stderr)
    report["linear"] = out
    return out


def stage_catboost(df, weights, report):
    try:
        from catboost import CatBoost, Pool
    except Exception as e:  # pragma: no cover
        report["catboost"] = {"error": str(e)}
        print(f"# catboost unavailable: {e}", file=sys.stderr)
        return None
    tune, test = window(df, TUNE), window(df, TEST)
    cols = features_of(df, with_ext=False)
    fold = tune.tournament_id.map(folds_of(tune.tournament_id.unique()))
    va = tune[fold == 0]
    tr = pool(df, set(va.tournament_id))

    def mk(g):
        return Pool(
            g[cols].to_numpy(dtype=np.float32),
            label=g.label.to_numpy(dtype=np.float32),
            group_id=g.tournament_id.to_numpy(),
            weight=g.tournament_id.map(lambda t: weights.get(int(t), 0.0)).to_numpy(),
        )

    m = CatBoost({
        "loss_function": "YetiRank", "iterations": 800, "learning_rate": 0.03,
        "depth": 6, "verbose": False, "random_seed": SEED, "thread_count": 8,
    })
    m.fit(mk(tr), eval_set=mk(va), early_stopping_rounds=80)
    rv = per_tournament(va, pd.Series(m.predict(mk(va)), index=va.index))
    rt = per_tournament(test, pd.Series(m.predict(mk(test)), index=test.index))
    b = per_tournament(va, va.m_forecast)
    report["catboost"] = {
        "fold0": wmean(rv, weights), "fold0_round3": wmean(b, weights),
        "test": wmean(rt, weights), "test3": wmean(rt, weights, k=1),
        "n_features": len(cols), "trees": int(m.tree_count_),
    }
    print(f"# catboost fold0 {wmean(rv, weights):.4f} against round 3 "
          f"{wmean(b, weights):.4f}, test {wmean(rt, weights):.4f}", file=sys.stderr)
    return report["catboost"]


# --------------------------------------------------------------------------- #
# the tables of out/discovery/report.md


def runs_of(d):
    return {int(k): v for k, v in d.items()}


def stage_report(path, weights):
    """Every table of the report, from a finished `rank.json`."""
    with open(path, encoding="utf-8") as f:
        r = json.load(f)
    p = print
    c = r.get("ceiling", {})
    names = {
        "round3": "round 3, the shipped forecast",
        "ranker": "the ranker, no external features",
        "ranker_ext": "the ranker, with them",
        "ranker_base": "the ranker started from round 3",
        "ranker_base_ext": "the same, with the external features",
        "fun": "rating.chgk.fun",
        "quest": "chgk.quest",
    }
    if c:
        p("| system | tune | @3 | tune, unweighted | @3 | test | @3 | test, unweighted | @3 | fields |")
        p("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
        for k, label in names.items():
            if k not in c:
                continue
            a, b = runs_of(c[k]["tune"]), runs_of(c[k]["test"])
            p(
                f"| {label} | {wmean(a, weights):.4f} | {wmean(a, weights, k=1):.4f} "
                f"| {wmean(a, None):.4f} | {wmean(a, None, k=1):.4f} "
                f"| {wmean(b, weights):.4f} | {wmean(b, weights, k=1):.4f} "
                f"| {wmean(b, None):.4f} | {wmean(b, None, k=1):.4f} "
                f"| {len(a)} / {len(b)} |"
            )
        p("")
        p("Paired bootstrap over the fields both rows cover, 2000 resamples, weighted.")
        p("")
        p("| a | b | window | NDCG | SE | NDCG@3 | SE | fields |")
        p("|---|---|---|---:|---:|---:|---:|---:|")
        for a_k in ("ranker", "ranker_ext", "ranker_base", "ranker_base_ext"):
            if a_k not in c:
                continue
            for b_k in ("round3", "fun", "quest"):
                if b_k not in c:
                    continue
                for wnd in ("tune", "test"):
                    d, se, n = paired(runs_of(c[a_k][wnd]), runs_of(c[b_k][wnd]), weights)
                    d3, se3, _ = paired(
                        runs_of(c[a_k][wnd]), runs_of(c[b_k][wnd]), weights, k=1
                    )
                    p(
                        f"| {names[a_k]} | {names[b_k]} | {wnd} | {d:+.4f} | {se:.4f} "
                        f"| {d3:+.4f} | {se3:.4f} | {n} |"
                    )
    imp = r.get("importance")
    if imp:
        p("")
        gr = {c_: i + 1 for i, c_ in enumerate(sorted(imp["gain"], key=lambda x: -imp["gain"][x]))}
        sr = {c_: i + 1 for i, c_ in enumerate(sorted(imp["shap"], key=lambda x: -imp["shap"][x]))}
        p(f"Out-of-fold weighted NDCG of the model the importances are read off: "
          f"{imp['base']:.4f}.")
        p("")
        p("| # | feature | family | permutation | gain rank | SHAP rank |")
        p("|---:|---|---|---:|---:|---:|")
        for i, c_ in enumerate(r["perm_order"][:25]):
            fam = next((k for k, v in FAMILIES.items() if c_.startswith(v)), "")
            p(f"| {i + 1} | `{c_}` | {fam} | {imp['perm'][c_]:+.5f} | {gr[c_]} | {sr[c_]} |")
    ab = r.get("ablation")
    if ab:
        p("")
        p("| features | columns | tune NDCG | @3 | cost |")
        p("|---|---:|---:|---:|---:|")
        base = ab["all"][0]
        for k, v in ab.items():
            cost = "-" if k == "all" else f"{v[0] - base:+.4f}"
            p(f"| {k} | {v[2]} | {v[0]:.4f} | {v[1]:.4f} | {cost} |")
    fw = r.get("forward")
    if fw:
        p("")
        p("| features | tune NDCG | @3 | test NDCG | @3 | the columns |")
        p("|---:|---:|---:|---:|---:|---|")
        for k in sorted(fw, key=int):
            v = fw[k]
            p(f"| {k} | {v['tune']:.4f} | {v['tune3']:.4f} | {v['test']:.4f} "
              f"| {v['test3']:.4f} | {', '.join('`' + x + '`' for x in v['cols'])} |")
    lin = r.get("linear")
    if lin:
        p("")
        p(f"Pairwise logistic on {lin['pairs']} pairs of the tune window, features "
          f"standardised.")
        p("")
        p("| feature | coefficient |")
        p("|---|---:|")
        for c_, b in zip(lin["cols"], lin["beta"]):
            p(f"| `{c_}` | {b:+.3f} |")
        p("")
        p(f"tune {lin['tune']:.4f} @3 {lin['tune3']:.4f}, "
          f"test {lin['test']:.4f} @3 {lin['test3']:.4f}")
        if c:
            for b_k in ("round3", "fun"):
                if b_k in c:
                    for wnd in ("tune", "test"):
                        d, se, n = paired(
                            runs_of(lin[wnd + "_runs"]), runs_of(c[b_k][wnd]), weights
                        )
                        p(f"  linear minus {names[b_k]}, {wnd}: {d:+.4f} se {se:.4f} ({n})")
    cb = r.get("catboost")
    if cb:
        p("")
        p(f"CatBoost YetiRank: {cb}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--features", default=f"{OUT}/features.db")
    ap.add_argument("--stage", default="all")
    ap.add_argument("--out", default=f"{OUT}/rank.json")
    a = ap.parse_args()

    weights = N.load_weights(N.WEIGHTS)
    if a.stage == "report":
        stage_report(a.out, weights)
        return

    import lightgbm as lgb

    df = load(a.features)
    print(
        f"# {len(df)} results in {df.tournament_id.nunique()} fields; "
        f"{len(features_of(df, False))} features, "
        f"{len(features_of(df, True))} with the external ones",
        file=sys.stderr,
    )
    os.makedirs(OUT, exist_ok=True)
    report = {}
    all_stages = ["ceiling", "importance", "ablation", "forward", "linear", "catboost"]
    stages = all_stages if a.stage == "all" else a.stage.split(",")

    if "ceiling" in stages:
        stage_ceiling(lgb, df, weights, report)
    if "importance" in stages:
        stage_importance(lgb, df, weights, report)
    order = report.get("perm_order")
    if "ablation" in stages:
        stage_ablation(lgb, df, weights, report)
    if order:
        # The ranker may lean on an external column; the distillation may not, so
        # the forward curve and the linear model are over what could ship.
        own = [c for c in order if not c.startswith("ext_")]
        if "forward" in stages:
            stage_forward(lgb, df, weights, report, own)
        if "linear" in stages:
            stage_linear(df, weights, report, own[:8])
    if "catboost" in stages:
        stage_catboost(df, weights, report)

    def plain(o):
        if isinstance(o, dict):
            return {str(k): plain(v) for k, v in o.items()}
        if isinstance(o, (list, tuple)):
            return [plain(x) for x in o]
        if isinstance(o, (np.floating, np.integer)):
            return o.item()
        return o

    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(plain(report), f)
    print(f"# wrote {a.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
