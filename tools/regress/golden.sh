#!/usr/bin/env bash
# golden.sh BIN OUT [steps...]: run the pipeline and lab binaries from BIN into OUT.
set -u
BIN=$(realpath "$1"); OUT=$2; shift 2
STEPS=${*:-"fit eval achieve webindex anomaly calibrate sweep simple fun gg quest"}
cd "$(git rev-parse --show-toplevel)"
mkdir -p "$OUT"
TO=2026-10-08
run() { local name=$1; shift; local t0=$SECONDS; if "$@" > "$OUT/$name.out" 2> "$OUT/$name.err"; then echo "ok   $name $((SECONDS-t0))s"; else echo "FAIL $name $((SECONDS-t0))s"; tail -3 "$OUT/$name.err"; fi; }
for s in $STEPS; do case $s in
  fit) run fit $BIN/fit --to $TO --rows-every 13 --out $OUT/ratings.db ;;
  eval) run eval $BIN/eval --db $OUT/ratings.db; run ndcg $BIN/eval --db $OUT/ratings.db --ndcg ;;
  achieve) run achieve_build $BIN/achieve --build --to $TO --dir $OUT/achieve --top 1
           cp $OUT/ratings.db $OUT/ratings_pub.db; run achieve_pub $BIN/achieve --to $TO --dir $OUT/achieve --publish $OUT/ratings_pub.db ;;
  webindex) run webindex $BIN/webindex --ratings $OUT/ratings_pub.db --out $OUT/web.db --rebuild ;;
  anomaly) run anomaly $BIN/anomaly --db $OUT/ratings_pub.db --web $OUT/web.db ;;
  calibrate) run calibrate $BIN/calibrate --db $OUT/ratings.db ;;
  sweep) run sweep $BIN/team_sweep --to 2026-09-05 --boot 50 --axis k_r=0.05 --axis two_part=true --point place_sign=false ;;
  simple) run simple $BIN/simple --rows-every 0 --out $OUT/simple.db ;;
  fun) run fun $BIN/ext_fun --out $OUT/fun.db ;;
  gg) run gg $BIN/ext_gg --to $TO --rows-every 0 --out $OUT/gg.db ;;
  quest) run quest $BIN/ext_quest --to $TO --out $OUT/quest.db ;;
esac; done
