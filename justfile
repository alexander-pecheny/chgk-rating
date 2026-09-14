# The production checkout; `just deploy` refits and restarts the site there.
prod := "/home/pecheny/new_chgk_rating"

# Build both crates here.
build:
    cargo build --release
    cargo build --release --manifest-path web/Cargo.toml

# Fast-forward master in the production checkout to this branch and build there.
ship:
    git -C {{prod}} merge --ff-only $(git rev-parse --abbrev-ref HEAD)
    cargo build --release --manifest-path {{prod}}/Cargo.toml
    cargo build --release --manifest-path {{prod}}/web/Cargo.toml

# Sync buff.db, fetch team names, refit, eval, restart the site (what cron runs at 06:30 UTC).
deploy:
    REPO={{prod}} {{prod}}/deploy/update.sh

# Refit and restart without the rsync and the name fetch.
redeploy:
    REPO={{prod}} SKIP_SYNC=1 {{prod}}/deploy/update.sh

# Score this checkout's out/final.db against every other system.
compare db="out/final.db":
    uv run tools/compare_ndcg.py {{db}}
