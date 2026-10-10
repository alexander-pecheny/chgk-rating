# Everything that is done by hand, in one place. `just` with no recipe lists them.
# The live site is a2.pecheny.me, served by the chgk-rating unit out of
# /home/pecheny/new_chgk_rating; see deploy/README.md.

repo := justfile_directory()

default:
    @just --list

# Build the model binaries and the site server.
build: web
    cargo build --release

# Build the Go site server into web/serve.
web:
    cd web && go build -o serve .

# Regenerate the Catalog after editing web/i18nstrings/ru/*.toml.
strings:
    cd web/i18nstrings && go generate .

# Rebuild the methodology page from the post after editing docs/post.md.
method:
    uv run tools/method_html.py docs/post.md web/method.html

# Regenerate the typed builder after editing web/ui/vocab.json.
ui:
    cd web/ui && go generate .

# Go tests and vet for the site server.
test-web:
    cd web && go vet ./... && go test ./...

# Publish this checkout to a2.pecheny.me (about five minutes).
deploy: build
    deploy/update.sh

# The same, but keep the buff.db that is already here.
deploy-here: build
    SKIP_SYNC=1 deploy/update.sh

# A rehearsal: fit and evaluate in this directory, touch no service.
deploy-dry: build
    SKIP_SYNC=1 SKIP_RESTART=1 REPO="{{repo}}" deploy/update.sh

# Fast-forward main in the production checkout to this branch, build there, publish.
# From a worktree; `just ship` then `just deploy`.
prod := env_var_or_default("PROD", "/home/pecheny/new_chgk_rating")
ship:
    git -C {{prod}} merge --ff-only $(git rev-parse --abbrev-ref HEAD)
    cargo build --release --manifest-path {{prod}}/Cargo.toml
    cd {{prod}}/web && go build -o serve .

# The last lines of the daily log, and whether the site is up.
status:
    @tail -n 12 out/update.log 2>/dev/null || echo "no out/update.log yet"
    @systemctl is-active chgk-rating || true
    @curl -sf -o /dev/null -w 'site %{http_code} in %{time_total}s\n' http://127.0.0.1:9684/ || echo "site did not answer"

# One full fit into out/ratings.db with the shipped config.
fit:
    ./target/release/fit --config config.toml

# The acceptance report, and the forecast metric on its own.
report:
    ./target/release/eval > out/eval.md
    ./target/release/eval --ndcg

# The forecast metric of every system, both ways of ordering a field.
compare:
    uv run tools/compare_ndcg.py --list

# Where the forecast still loses: the worst fields, the features, the rivals.
diagnose db="out/ratings.db":
    uv run tools/q_diag.py {{db}} --teams 6 --cohorts
    uv run tools/q_feat.py {{db}}
    uv run tools/q_vs.py {{db}} out/ext/fun.db

# One config sweep. `just sweep out/q_ndcg/batch2/glue.txt out/q2`.
sweep batch dir="out/q_ndcg":
    uv run tools/q_ndcg_sweep.py --dir {{dir}} --jobs 3 {{batch}}

# Rebuild the rules report; needs the fonts of tools/fetch/fonts.py.
rules:
    typst compile --font-path fonts report/report.typ report/report.pdf

# The maintainability gate: sloplint's score must stay under sloplint.toml's ceiling.
slop:
    sloplint --format summary .

# Run a build's binaries through every step into a directory, for tools/regress/cmp.py.
regress bin out:
    tools/regress/golden.sh {{bin}} {{out}}
    tools/regress/variants.sh {{bin}} {{out}}/var
