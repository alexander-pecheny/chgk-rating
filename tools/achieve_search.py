# /// script
# dependencies = ["pandas", "numpy", "scikit-learn"]
# ///
"""What the achievement knob search found.

Reads `achieve --search`'s table, fits a gradient-boosted surrogate of tune
NDCG on the knobs, and prints: how well the surrogate predicts held-out
points, each knob's permutation importance, the partial dependence of the
important ones, and the best configurations by tune with their test scores.

    uv run tools/achieve_search.py out/achieve/search.tsv
"""

import argparse

import numpy as np
import pandas as pd
from sklearn.ensemble import HistGradientBoostingRegressor
from sklearn.inspection import permutation_importance
from sklearn.model_selection import train_test_split

CATS = ["perf", "field_mode", "fade", "overlap_by", "field_k", "base_rule"]
HELD_OUT = 0.2
SURROGATE = {"max_iter": 600, "learning_rate": 0.05, "categorical_features": "from_dtype", "random_state": 0}
REPEATS = 8
TOP_DECILE = 0.9
SHOWN = 10
# The partial-dependence grid of a numeric knob: seven quantiles, trimmed.
GRID = np.linspace(0.02, 0.98, 7)
GRID_DIGITS = 3


def knob_frame(d, knobs):
    x = d[knobs].copy()
    for c in knobs:
        x[c] = x[c].astype(str).astype("category") if c in CATS else pd.to_numeric(x[c])
    return x


def fit(x, y):
    """The surrogate, and the held-out split it is judged on."""
    xtr, xte, ytr, yte = train_test_split(x, y, test_size=HELD_OUT, random_state=0)
    return HistGradientBoostingRegressor(**SURROGATE).fit(xtr, ytr), xte, yte


def correlations(d, r2):
    print(f"{len(d)} configurations; surrogate R² on held-out points {r2:.3f}")
    print(f"tune/test correlation over all points {np.corrcoef(d.tune, d.test)[0, 1]:.3f}")
    top = d[d.tune >= d.tune.quantile(TOP_DECILE)]
    print(f"tune/test correlation over the top decile {np.corrcoef(top.tune, top.test)[0, 1]:.3f}\n")


def importance(m, xte, yte, knobs):
    """Knobs by permutation importance, most important first."""
    imp = permutation_importance(m, xte, yte, n_repeats=REPEATS, random_state=0)
    order = np.argsort(-imp.importances_mean)
    print("| knob | importance (drop in R²) |\n|---|---|")
    for i in order:
        print(f"| {knobs[i]} | {imp.importances_mean[i]:.4f} |")
    return order


def partial_dependence(m, x, k):
    """Mean prediction with knob `k` set to each grid value for every point."""
    grid = list(x[k].cat.categories) if k in CATS else np.unique(np.quantile(x[k], GRID).round(GRID_DIGITS))
    cells = []
    for g in grid:
        xx = x.copy()
        xx[k] = pd.Categorical([g] * len(xx), categories=x[k].cat.categories) if k in CATS else g
        cells.append(f"{g}: {m.predict(xx).mean():.4f}")
    return cells


def best(d, knobs):
    print("\nBest ten by tune:")
    for _, r in d.sort_values("tune", ascending=False).head(SHOWN).iterrows():
        print(f"tune {r.tune:.4f} test {r.test:.4f} [{r.phase}] " + ", ".join(f"{k}={r[k]}" for k in knobs))
    ref = d[d.phase == "default"].iloc[0]
    print(f"\ndefault: tune {ref.tune:.4f} test {ref.test:.4f}")


def main(path: str) -> None:
    d = pd.read_csv(path, sep="\t")
    knobs = [c for c in d.columns if c not in ("phase", "tune", "test")]
    x = knob_frame(d, knobs)
    m, xte, yte = fit(x, d["tune"].to_numpy())
    correlations(d, m.score(xte, yte))
    order = importance(m, xte, yte, knobs)
    print("\nPartial dependence of the ten most important knobs (mean predicted tune NDCG):")
    for i in order[:SHOWN]:
        print(f"- {knobs[i]}: " + ", ".join(partial_dependence(m, x, knobs[i])))
    best(d, knobs)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("path", nargs="?", default="out/achieve/search.tsv")
    main(ap.parse_args().path)
