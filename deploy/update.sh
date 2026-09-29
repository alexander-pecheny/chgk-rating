#!/bin/sh
# The daily refit: pull the new buff.db, fit, eval, restart the site.
# Cron runs it at 06:30 UTC and appends the output to out/update.log.
# See deploy/README.md.
set -eu

REPO=${REPO:-/home/pecheny/new_chgk_rating}
cd "$REPO"
mkdir -p out

# cron gives us almost no PATH.
PATH="$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin"
export PATH

log() {
    echo "$(date -u '+%Y-%m-%d %H:%M:%S') $*"
}

# Two runs must never overlap; the second one leaves at once.
exec 9>"$REPO/out/update.lock"
if ! flock -n 9; then
    log "another run holds out/update.lock; nothing to do"
    exit 0
fi

started=$(date -u '+%s')
log "update starts in $REPO"

if [ "${SKIP_SYNC:-}" = "1" ]; then
    log "sync skipped"
else
    log "sync: rsync vps2day-ee:/home/ap/buff/buff.db"
    # Yesterday's copy is the basis rsync compares against, so only the pages
    # that changed cross the network: about 10 MB of the 1 GB, in seconds. The
    # copy is made beside it, so a failed sync never touches the good buff.db.
    [ -f "$REPO/buff.db" ] && cp "$REPO/buff.db" "$REPO/buff.db.tmp"
    if rsync -a --inplace --no-whole-file vps2day-ee:/home/ap/buff/buff.db "$REPO/buff.db.tmp"; then
        mv -f "$REPO/buff.db.tmp" "$REPO/buff.db"
        log "sync done"
    else
        rm -f "$REPO/buff.db.tmp"
        log "sync failed; the old buff.db stays and the run stops here"
        exit 1
    fi
fi

log "fit"
if ! ./target/release/fit > out/fit.log 2>&1; then
    log "fit failed, see out/fit.log; the site keeps the old numbers"
    tail -n 20 out/fit.log
    exit 1
fi
log "fit done: $(tail -n 1 out/fit.log)"

log "eval"
if ! ./target/release/eval > out/eval.md 2>&1; then
    log "eval failed, see out/eval.md; the site keeps the old numbers"
    tail -n 20 out/eval.md
    exit 1
fi
log "eval done"

# The team rating the site publishes is the achievement rating: its own player
# pass freezes every result's value, then it replaces the team rows of the fit.
log "achieve"
if ! ./target/release/achieve --build --top 1 > out/achieve.log 2>&1; then
    log "achieve failed, see out/achieve.log; the site keeps the old numbers"
    tail -n 20 out/achieve.log
    exit 1
fi
if ! ./target/release/achieve --publish out/ratings.db >> out/achieve.log 2>&1; then
    log "achieve publish failed, see out/achieve.log"
    tail -n 20 out/achieve.log
    exit 1
fi
log "achieve done: $(tail -n 1 out/achieve.log)"

if [ "${SKIP_RESTART:-}" = "1" ]; then
    log "restart skipped"
else
    log "restart chgk-rating"
    sudo -n systemctl restart chgk-rating
    # The serving index is rebuilt on start, about 90 seconds. Wait three minutes.
    waited=0
    while [ "$waited" -lt 180 ]; do
        if curl -sf -o /dev/null http://127.0.0.1:9684/; then
            break
        fi
        sleep 5
        waited=$((waited + 5))
    done
    if [ "$waited" -lt 180 ]; then
        log "site answered after ${waited}s"
    else
        log "site did not answer within three minutes"
    fi
fi

size=$(du -Lh "$REPO/buff.db" | cut -f1)
release=$(sqlite3 out/ratings.db 'select max(date) from releases' 2>/dev/null || echo '?')
log "done: buff.db $size, last release $release, $(($(date -u '+%s') - started))s"
