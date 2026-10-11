#!/bin/sh
# The daily refit: pull the new buff.db, fit, eval, restart the site.
# Cron runs it at 06:30 UTC and appends the output to out/update.log.
# See deploy/README.md.
set -eu

# The checkout this script lives in, unless REPO says otherwise.
REPO=${REPO:-$(cd "$(dirname "$0")/.." && pwd)}
# Where the mirror is rebuilt nightly; any rsync source works.
BUFF_SOURCE=${BUFF_SOURCE:-vps2day-ee:/home/ap/buff/buff.db}
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
    log "sync: rsync $BUFF_SOURCE"
    # Yesterday's copy is the basis rsync compares against, so only the pages
    # that changed cross the network: about 10 MB of the 1 GB, in seconds. The
    # copy is made beside it, so a failed sync never touches the good buff.db.
    [ -f "$REPO/buff.db" ] && cp "$REPO/buff.db" "$REPO/buff.db.tmp"
    if rsync -a --inplace --no-whole-file "$BUFF_SOURCE" "$REPO/buff.db.tmp"; then
        mv -f "$REPO/buff.db.tmp" "$REPO/buff.db"
        log "sync done"
    else
        rm -f "$REPO/buff.db.tmp"
        log "sync failed; the old buff.db stays and the run stops here"
        exit 1
    fi
fi

# Every stage is a function of its inputs, so a stage whose inputs have not
# changed since it last succeeded is skipped, the way make or Typst skip work
# that is current. A stage's key hashes its own binary and everything it reads,
# including the keys of the stages it reads from; out/stamps/<stage> holds the
# key of its last success. FORCE=1 runs everything.
hash() { sha256sum "$@" | sha256sum | cut -c1-16; }
fresh() { [ "${FORCE:-}" != "1" ] && [ "$(cat "out/stamps/$1" 2>/dev/null)" = "$2" ]; }
# A stage clears its stamp before it starts, so one that dies halfway is stale.
unstamp() { mkdir -p out/stamps; : > "out/stamps/$1"; }
stamp() { echo "$2" > "out/stamps/$1"; }

bin=target/release
k_buff=$(hash buff.db)
k_data=$(hash config*.toml $(find -L data -type f | sort))
k_fit=$(echo "$k_buff $k_data $(hash $bin/fit)" | hash -)
k_eval=$(echo "$k_fit $(hash $bin/eval)" | hash -)
k_pub=$(echo "$k_fit $(hash $bin/achieve)" | hash -)
# Each run's window ends before the day after buff.db's last complete update,
# not before the clock's today: the same data gives the same numbers on any day.
today=$(sqlite3 buff.db "select date(substr(d,1,19), printf('%+d minutes', (case substr(d,20,1) when '-' then 1 else -1 end) * (substr(d,21,2) * 60 + substr(d,23,2))), '+1 day') from (select max(datetime) d from db_updates)")

# The fit also freezes the achievement rating's values on its own pass, which
# achieve would otherwise replay. Eval reads the fit's own team rows, which
# the publish below replaces, so a stale eval refits as well.
if fresh fit "$k_fit" && fresh eval "$k_eval"; then
    log "fit and eval are current"
else
    log "fit"
    unstamp fit
    unstamp eval
    unstamp publish
    if ! $bin/fit --achieve out/achieve > out/fit.log 2>&1; then
        log "fit failed, see out/fit.log; the site keeps the old numbers"
        tail -n 20 out/fit.log
        exit 1
    fi
    stamp fit "$k_fit"
    log "fit done: $(tail -n 1 out/fit.log)"

    log "eval"
    if ! $bin/eval > out/eval.md 2>&1; then
        log "eval failed, see out/eval.md; the site keeps the old numbers"
        tail -n 20 out/eval.md
        exit 1
    fi
    stamp eval "$k_eval"
    log "eval done"
fi

# The team rating the site publishes is the achievement rating: from the values
# the fit froze, it replaces the team rows of the fit.
if fresh publish "$k_pub"; then
    log "achieve is current"
else
    log "achieve"
    unstamp publish
    if ! $bin/achieve --publish out/ratings.db > out/achieve.log 2>&1; then
        log "achieve publish failed, see out/achieve.log"
        tail -n 20 out/achieve.log
        exit 1
    fi
    stamp publish "$k_pub"
    log "achieve done: $(tail -n 1 out/achieve.log)"
fi

# Our reimplementations of the other systems, for the forecast comparison on
# tournament pages. A failure here costs that comparison, not the update. They
# read our player ratings, so the fit's key is part of theirs. The three are
# independent and each mostly one thread, so they run at once.
log "rivals"
mkdir -p out/ext
rival() {
    name=$1
    shift
    k=$(echo "$k_fit $(hash "$bin/$name") $*" | hash -)
    if fresh "$name" "$k"; then
        return
    fi
    unstamp "$name"
    if "$bin/$name" "$@" > "out/rivals_$name.log" 2>&1; then
        stamp "$name" "$k"
    else
        log "rival $name failed, see out/rivals_$name.log; its comparison keeps yesterday's numbers"
    fi
}
rival ext_fun --as-of "$today" &
rival ext_gg --to "$today" --rows-every 0 &
rival ext_quest --to "$today" &
wait
log "rivals done"

# The serving index: names, rosters, ranking tables and anomaly scores. It is
# built beside the old one and renamed in, so the running site never reads half
# of it.
k_web=$(echo "$k_pub $k_eval $(cat out/stamps/ext_fun out/stamps/ext_gg out/stamps/ext_quest 2>/dev/null) $(hash $bin/webindex)" | hash -)
if fresh webindex "$k_web"; then
    log "webindex is current"
    SKIP_RESTART=${SKIP_RESTART:-1}
else
    log "webindex"
    unstamp webindex
    # --rebuild: the key above, not webindex's own look at file times, says
    # whether the index is current.
    if ! $bin/webindex --rebuild > out/webindex.log 2>&1; then
        log "webindex failed, see out/webindex.log; the site keeps the old numbers"
        tail -n 20 out/webindex.log
        exit 1
    fi
    stamp webindex "$k_web"
    log "webindex done: $(tail -n 1 out/webindex.log)"
fi

if [ "${SKIP_RESTART:-}" = "1" ]; then
    log "restart skipped"
else
    log "restart chgk-rating"
    sudo -n systemctl restart chgk-rating
    # The server only opens the databases, so it answers within seconds.
    waited=0
    while [ "$waited" -lt 60 ]; do
        if curl -sf -o /dev/null http://127.0.0.1:9684/; then
            break
        fi
        sleep 5
        waited=$((waited + 5))
    done
    if [ "$waited" -lt 60 ]; then
        log "site answered after ${waited}s"
    else
        log "site did not answer within a minute"
    fi
fi

size=$(du -Lh "$REPO/buff.db" | cut -f1)
release=$(sqlite3 out/ratings.db 'select max(date) from releases' 2>/dev/null || echo '?')
log "done: buff.db $size, last release $release, $(($(date -u '+%s') - started))s"
