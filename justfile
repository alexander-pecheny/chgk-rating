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

# The variants /method/'s metrics block sets beside the daily outputs: every
# system trained on the official rating's tournaments alone and on all of them,
# each forecasting every tournament from 2025 on, trained on or not. Trained on
# all, the rivals are the update's own out/ext dbs, and chgk.gg's official
# rating, which moves on rated tournaments alone, is the rated one. Run after an
# update.
metrics-fit:
    #!/bin/sh
    set -eu
    d=out/metrics b=target/release
    mkdir -p $d/all $d/rated
    today=$(sqlite3 buff.db "select date(substr(max(datetime),1,10), '+1 day') from db_updates")
    from=2025-01-01
    rated() { sed -e 's/^w_async = .*/w_async = 0.0/' -e 's/^include_unrated = .*/include_unrated = false/'; }
    one() { sed -e "s|^out_path = .*|out_path = \"$1\"|" -e 's/^rows_every = .*/rows_every = 0/' -e "/^\[data\]/a forecast_from = \"$from\""; }
    blend() { printf '\n[blend]\nelo_db = "%s"\nelo = 0.35\nplayers = 0.35\n' $1; }
    all() { sed -e 's/^w_async = .*/w_async = 0.25/' -e 's/^train_unrated = .*/train_unrated = true/'; }
    one $d/rated/a2.db < config.toml > $d/a2_rated.toml
    one $d/all/a2.db < config.toml | all > $d/a2_all.toml
    { one $d/rated/blendt.db < config.toml; blend $d/rated/elot.db; } > $d/blendt_rated.toml
    { one $d/all/blendt.db < config.toml | all; blend out/ext/elot.db; } > $d/blendt_all.toml
    rated < config_elo.toml > $d/elo_rated.toml
    one $d/rated/quest.db < config_quest.toml | rated > $d/quest_rated.toml
    # A rival reads neither config.toml nor A2's output: it reruns only when
    # its output is older than buff.db, its binary or the rival configs.
    rival() {
        out=$1 bin=$2 log=$3; shift 3
        for dep in buff.db $b/$bin config_*.toml; do
            [ -e "$out" ] && [ "$out" -nt "$dep" ] || { $b/$bin "$@" > $log 2>&1; return; }
        done
    }
    ff="--forecast-from $from"
    rival $d/rated/elo.db ext_elo $d/elo.log --config $d/elo_rated.toml --to $today --out $d/rated/elo.db $ff &
    rival $d/rated/elot.db ext_elo $d/elot.log --config $d/elo_rated.toml --to $today --out $d/rated/elot.db $ff --young 150 --ip-new 25 &
    rival $d/rated/fun.db ext_fun $d/fun.log --as-of $today --rated-only --out $d/rated/fun.db $ff &
    rival $d/rated/quest.db ext_quest $d/quest.log --config $d/quest_rated.toml --to $today $ff &
    rival $d/all/gg.db ext_gg $d/gg.log --to $today --rows-every 0 --move-all --out $d/all/gg.db &
    $b/fit --config $d/a2_rated.toml > $d/a2_rated.log &
    $b/fit --config $d/a2_all.toml > $d/a2_all.log &
    $b/fit --config $d/blendt_all.toml > $d/blendt_all.log &
    wait
    $b/fit --config $d/blendt_rated.toml > $d/blendt_rated.log

# /method/'s metrics block, web/metrics.json: the systems by the tournaments
# they train on and those they are scored on, each against every other. Only
# in-person tournaments carry weights.
metrics:
    #!/bin/sh
    set -eu
    d=out/metrics
    run() {
        train=$1 set=$2 how=; shift 2
        [ "$1" = u ] && how=--unweighted
        shift
        uv run tools/rival_metrics.py --json --from 2025-01-01 --to 2026-08-28 --set $set $how "$@" | jq -c --arg t "$train" --arg s "$set" '. + {train: $t, set: $s}' | sed 's/A2_1/A2 1/g'
    }
    all="A2_1.4=$d/all/a2.db A2+Elo+tweaks=$d/all/blendt.db elo-chgk.uk=out/ext/elo.db:x elo-chgk.uk+tweaks=out/ext/elot.db:x chgk.fun=out/ext/fun.db chgk.quest=out/ext/quest.db chgk.gg=$d/all/gg.db"
    rated="A2_1.4=$d/rated/a2.db A2+Elo+tweaks=$d/rated/blendt.db elo-chgk.uk=$d/rated/elo.db:x elo-chgk.uk+tweaks=$d/rated/elot.db:x chgk.fun=$d/rated/fun.db chgk.quest=$d/rated/quest.db chgk.gg=out/ext/gg.db"
    k=0
    for train in rated all; do
        eval specs=\$$train
        for v in "ip-rated w" "ip-rated u" "rated u" "all u"; do
            k=$((k + 1))
            run $train $v $specs > $d/t$k.json &
        done
    done
    wait
    cat $(seq -f "$d/t%g.json" 1 $k) | jq -s . > web/metrics.json

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
