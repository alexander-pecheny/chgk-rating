# Regression check

A refactor must leave every output the same. Build the code before and after,
run both builds through the same steps, and compare.

```
cargo build --release && mkdir -p out/ref/bin && cp target/release/* out/ref/bin/ 2>/dev/null
tools/regress/golden.sh out/ref/bin out/ref/run          # all steps; or name some: fit eval
tools/regress/variants.sh out/ref/bin out/ref/var        # the rules config.toml leaves off
# ... change the code, build, copy to out/new/bin, run the same two commands into out/new ...
uv run python tools/regress/cmp.py out/ref/run out/new/run
uv run python tools/regress/cmp.py out/ref/var out/new/var
```

`golden.sh` runs fit, eval, achieve, webindex, anomaly, calibrate, the team
sweep, simple and the three rival engines on a fixed window. `cmp.py` compares
every table of every database inside SQLite, so a table larger than memory
works, and every text output line by line, ignoring timings and paths. It
prints SAME or each difference. The whole run takes about fifteen minutes and a
few GB under `out/`.

Two numbers are known to differ between any two runs of the same build: the
bootstrap standard error in the team sweep, and the order of tied players in
the anomaly report and of the site's search index.
