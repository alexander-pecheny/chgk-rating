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

# The fonts Typst sets /method/'s formulas in, as SVG on the server: readers
# get only the glyph outlines, so the files are Google Fonts' whole ones.
math-fonts:
    curl -sSfL -o web/math/NotoSansMath-Regular.ttf https://github.com/google/fonts/raw/main/ofl/notosansmath/NotoSansMath-Regular.ttf
    curl -sSfL -o web/math/NotoSans.ttf "https://github.com/google/fonts/raw/main/ofl/notosans/NotoSans%5Bwdth,wght%5D.ttf"

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

# It builds, fits into out/ from production's buff.db when out/ has no index
# yet or `refit` is given, and (re)starts the instance through ~/.local/bin/a2-dev.
# Serve this worktree at https://<name>.a2-dev.pecheny.me.
dev name refit="":
    #!/bin/sh
    set -eu
    [ -e buff.db ] || ln -s {{prod}}/buff.db buff.db
    [ -e data/external ] || ln -s {{prod}}/data/external data/external
    just build
    if [ -n "{{refit}}" ] || [ ! -e out/web.db ]; then
        SKIP_SYNC=1 SKIP_RESTART=1 REPO="{{repo}}" deploy/update.sh
    fi
    a2-dev add {{name}} "{{repo}}"

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

# The variants /method/'s metrics block sets beside the daily outputs: A2
# without the blend, every system on the official rating's tournaments alone,
# and chgk.gg with every tournament moving its ratings. Run after an update.
metrics-fit:
    #!/bin/sh
    set -eu
    d=out/metrics b=target/release
    mkdir -p $d/all $d/rated
    today=$(sqlite3 buff.db "select date(substr(max(datetime),1,10), '+1 day') from db_updates")
    rated() { sed -e 's/^w_async = .*/w_async = 0.0/' -e 's/^train_unrated = .*/train_unrated = false/' -e 's/^include_unrated = .*/include_unrated = false/'; }
    one() { sed -e "s|^out_path = .*|out_path = \"$1\"|" -e 's/^rows_every = .*/rows_every = 0/'; }
    sed '/^\[blend\]/,$d' config.toml | one $d/all/a2.db > $d/a2.toml
    sed '/^\[blend\]/,$d' config.toml | one $d/rated/a2.db | rated > $d/a2_rated.toml
    one $d/rated/blend.db < config.toml | sed "s|^elo_db = .*|elo_db = \"$d/rated/elo.db\"|" | rated > $d/blend_rated.toml
    rated < config_elo.toml > $d/elo_rated.toml
    one $d/rated/quest.db < config_quest.toml | rated > $d/quest_rated.toml
    $b/ext_elo --config $d/elo_rated.toml --to $today --out $d/rated/elo.db > $d/elo.log
    $b/fit --config $d/a2.toml > $d/a2.log & $b/fit --config $d/a2_rated.toml > $d/a2_rated.log & wait
    $b/fit --config $d/blend_rated.toml > $d/blend.log
    $b/ext_fun --as-of $today --rated-only --out $d/rated/fun.db > $d/fun.log 2>&1 &
    $b/ext_quest --config $d/quest_rated.toml --to $today > $d/quest.log 2>&1 &
    $b/ext_gg --to $today --rows-every 0 --move-all --out $d/all/gg.db > $d/gg.log 2>&1 &
    wait

# /method/'s metrics block, web/metrics.json: the systems on all tournaments
# and on rated ones alone, weighted and not, against the published blend.
metrics:
    #!/bin/sh
    set -eu
    d=out/metrics
    run() { set=$1 how=$2; shift 2; uv run tools/rival_metrics.py --json --from 2025-01-01 --to 2026-08-28 --paired A2+Сатыбалдыев $how "$@" | jq -c --arg s "$set" '. + {set: $s}'; }
    all="A2=$d/all/a2.db A2+Сатыбалдыев=out/ratings.db Сатыбалдыев=out/ext/elo.db:x Сироткин=out/ext/fun.db Фаттахов=out/ext/quest.db chgk.gg=$d/all/gg.db"
    rated="A2=$d/rated/a2.db A2+Сатыбалдыев=$d/rated/blend.db Сатыбалдыев=$d/rated/elo.db:x Сироткин=$d/rated/fun.db Фаттахов=$d/rated/quest.db chgk.gg=out/ext/gg.db"
    { run all "" $all; run all --unweighted $all; run rated "" $rated; run rated --unweighted $rated; } | jq -s . > web/metrics.json

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
