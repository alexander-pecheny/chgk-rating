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

import sys

import numpy as np
import pandas as pd
from sklearn.ensemble import HistGradientBoostingRegressor
from sklearn.inspection import permutation_importance
from sklearn.model_selection import train_test_split

CATS = ["perf", "field_mode", "fade", "overlap_by", "field_k", "base_rule"]


def main(path: str) -> None:
    d = pd.read_csv(path, sep="\t")
    knobs = [c for c in d.columns if c not in ("phase", "tune", "test")]
    x = d[knobs].copy()
    for c in knobs:
        if c in CATS:
            x[c] = x[c].astype(str).astype("category")
        else:
            x[c] = pd.to_numeric(x[c])
    y = d["tune"].to_numpy()
    xtr, xte, ytr, yte = train_test_split(x, y, test_size=0.2, random_state=0)
    m = HistGradientBoostingRegressor(
        max_iter=600, learning_rate=0.05, categorical_features="from_dtype", random_state=0
    ).fit(xtr, ytr)
    r2 = m.score(xte, yte)
    print(f"{len(d)} configurations; surrogate R² on held-out points {r2:.3f}")
    print(f"tune/test correlation over all points {np.corrcoef(d.tune, d.test)[0, 1]:.3f}")
    top = d[d.tune >= d.tune.quantile(0.9)]
    print(f"tune/test correlation over the top decile {np.corrcoef(top.tune, top.test)[0, 1]:.3f}\n")

    imp = permutation_importance(m, xte, yte, n_repeats=8, random_state=0)
    order = np.argsort(-imp.importances_mean)
    print("| knob | importance (drop in R²) |\n|---|---|")
    for i in order:
        print(f"| {knobs[i]} | {imp.importances_mean[i]:.4f} |")

    # Partial dependence: set one knob for every point, average the prediction.
    print("\nPartial dependence of the ten most important knobs (mean predicted tune NDCG):")
    for i in order[:10]:
        k = knobs[i]
        if k in CATS:
            grid = list(x[k].cat.categories)
        else:
            grid = np.unique(np.quantile(x[k], np.linspace(0.02, 0.98, 7)).round(3))
        cells = []
        for g in grid:
            xx = x.copy()
            xx[k] = pd.Categorical([g] * len(xx), categories=x[k].cat.categories) if k in CATS else g
            cells.append(f"{g}: {m.predict(xx).mean():.4f}")
        print(f"- {k}: " + ", ".join(cells))

    print("\nBest ten by tune:")
    best = d.sort_values("tune", ascending=False).head(10)
    for _, r in best.iterrows():
        diff = {k: r[k] for k in knobs}
        print(f"tune {r.tune:.4f} test {r.test:.4f} [{r.phase}] " + ", ".join(f"{k}={v}" for k, v in diff.items()))
    ref = d[d.phase == "default"].iloc[0]
    print(f"\ndefault: tune {ref.tune:.4f} test {ref.test:.4f}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "out/achieve/search.tsv")
