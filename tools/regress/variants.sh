#!/usr/bin/env bash
# variants.sh BIN OUT: the rule variants config.toml leaves off, on a short
# window, so a refactor of the team and player rules is checked off the
# shipped path too. Each variant is config.toml with a few switches turned on.
set -u
BIN=$(realpath "$1"); OUT=$2
cd "$(git rev-parse --show-toplevel)"
mkdir -p "$OUT"
run() { local name=$1; shift; local t0=$SECONDS; if "$@" > "$OUT/$name.out" 2> "$OUT/$name.err"; then echo "ok   $name $((SECONDS-t0))s"; else echo "FAIL $name $((SECONDS-t0))s"; tail -3 "$OUT/$name.err"; fi; }

# Eleven of the measured-and-off S2 switches at once.
sed -e 's/^rank_rule = "mean"/rank_rule = "erel"/; s/^tight_gap = 0.0/tight_gap = 40.0/' \
    -e 's/^sigma_c_tight = 1.5/sigma_c_tight = 1.0/; s/^kappa_s = 0.0/kappa_s = 50.0/' \
    -e 's/^home_bonus = 0.0/home_bonus = 20.0/; s/^load_7 = 0.0/load_7 = -5.0/; s/^chemistry = 0.0/chemistry = 10.0/' \
    -e 's/^difficulty_forecast = false/difficulty_forecast = true/; s/^tau_spread = 0.0/tau_spread = 0.1/' \
    -e 's/^sigma_v = 0.0/sigma_v = 0.1/; s/^q_sigma_c = 0.0/q_sigma_c = 0.5/; s/^sigma_ip = 0.0/sigma_ip = 0.5/' \
    -e 's/^margin_g = 0.0/margin_g = 5.0/; s/^\[team.s2\]$/[team.s2]\nphi_min = 10.0\nphi_spread = 5.0/' \
    config.toml > "$OUT/variant_s2_off_switches.toml"
# The two-part rating with a live core, information-normalised steps and field weights.
sed -e 's/^two_part = false/two_part = true/; s/^ed_phi = 0.0/ed_phi = 0.5/' \
    -e 's/^\[team.s2\]$/[team.s2]\ncore_live = true\ninfo_norm = true\nfield_gamma = 0.3\nfield_players = true/' \
    config.toml > "$OUT/variant_two_part.toml"
# The Kalman team engine S2 replaced.
sed -e 's/^engine = "s2"/engine = "kalman"/' config.toml > "$OUT/variant_kalman.toml"

for v in s2_off_switches two_part kalman; do
  run fit_$v $BIN/fit --config "$OUT/variant_$v.toml" --from 2016-01-01 --to 2019-06-01 --passes 1 --rows-every 0 --out "$OUT/variant_$v.db"
done
run sweep_all $BIN/team_sweep --to 2026-09-05 --boot 20 \
  --axis 'k_r=0.05;k_r_target=usual' --axis 'two_part=true;core_live=true' --axis 'two_part=true;core_top=true' \
  --axis best_n=5 --axis 'place_sign=false;winner_floor=false' --axis 'info_norm=true;margin_g=5' \
  --axis 'adj_mult=true;expect_forecast=true' --axis 'forecast_rating=true;field_gamma=0.3' --axis 'n0=10;start_frac=0.5'
run features $BIN/fit --from 2016-01-01 --to 2018-06-01 --passes 1 --rows-every 0 --out "$OUT/feat_fit.db" --features "$OUT/features.db"
