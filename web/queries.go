package main

import (
	"database/sql"
	"errors"
	"math"
	"sort"
	"strings"
)

// The listings: the two ranking tables, the tournaments, the releases, the
// anomaly scores and the search results.

func (st *store) number(query string, args ...any) (int, error) {
	var n int
	err := st.db.QueryRow(query, args...).Scan(&n)
	return n, err
}

type release struct {
	id   int64
	date string
}

func (r *release) fields() []any { return []any{&r.id, &r.date} }

// releases are every release, oldest first.
func (st *store) releases() ([]release, error) {
	return all[release](st.db, "select id, date from x.release_ order by date, id")
}

type teamRank struct {
	rank, id                       int64
	name, town                     *string
	display, sigma, evi, change    *float64
	prov                           bool
}

func (r *teamRank) fields() []any {
	return []any{&r.rank, &r.id, &r.name, &r.town, &r.display, &r.sigma, &r.evi, &r.change, &r.prov}
}

// teamRanks are the teams ranked after lo and up to hi.
func (st *store) teamRanks(lo, hi int) ([]teamRank, error) {
	return all[teamRank](st.db, `select k.rank, k.team_id, t.name, t.town, k.display, k.sigma, k.evidence, k.change,
		k.provisional from x.team_rank k left join x.team t on t.id = k.team_id
		where k.rank > ? and k.rank <= ? order by k.rank`, lo, hi)
}

type playerRank struct {
	rank, id       int64
	name           *string
	display, sigma *float64
	games          *int64
	change         *float64
	prov           bool
}

func (r *playerRank) fields() []any {
	return []any{&r.rank, &r.id, &r.name, &r.display, &r.sigma, &r.games, &r.change, &r.prov}
}

func (st *store) playerRanks(lo, hi int) ([]playerRank, error) {
	return all[playerRank](st.db, `select k.rank, k.player_id, p.name, k.display, k.sigma, k.games, k.change,
		k.provisional from x.player_rank k left join x.player p on p.id = k.player_id
		where k.rank > ? and k.rank <= ? order by k.rank`, lo, hi)
}

// firmTeams and firmPlayers count the settled ratings, which the tables list
// before the provisional ones.
func (st *store) firmTeams() (int, error) {
	return st.number("select count(*) from x.team_rank where provisional = 0")
}

func (st *store) firmPlayers() (int, error) {
	return st.number("select count(*) from x.player_rank where provisional = 0")
}

type tourLine struct {
	id                    int64
	name, date, typ       *string
	weight, truedl, llcf  *float64
	nTeams, nQ            *int64
}

func (r *tourLine) fields() []any {
	return []any{&r.id, &r.name, &r.date, &r.typ, &r.weight, &r.nTeams, &r.nQ, &r.truedl, &r.llcf}
}

func (st *store) tournamentCount() (int, error) {
	return st.number("select count(*) from x.tournament")
}

// tournaments are a page of tournaments, newest first.
func (st *store) tournaments(limit, offset int) ([]tourLine, error) {
	return all[tourLine](st.db, `select id, name, date, type, weight, n_teams, n_q, truedl, ll_cf from x.tournament
		order by date desc, id desc limit ? offset ?`, limit, offset)
}

type releaseLine struct {
	id, nPlayers, nTeams int64
	date                 string
	shift, cum           float64
}

func (r *releaseLine) fields() []any {
	return []any{&r.id, &r.date, &r.nPlayers, &r.nTeams, &r.shift, &r.cum}
}

func (st *store) releaseLines() ([]releaseLine, error) {
	return all[releaseLine](st.db, "select id, date, n_players, n_teams, shift, shift_cum from x.release_ order by date desc, id desc")
}

type anomalyLine struct {
	rank, id                                 int64
	name                                     *string
	prank                                    *int64
	display, k, mGap, mOn, mIP, fGap, fOn, fIP *float64
}

func (r *anomalyLine) fields() []any {
	return []any{&r.rank, &r.id, &r.name, &r.prank, &r.display, &r.k, &r.mGap, &r.mOn, &r.mIP, &r.fGap, &r.fOn, &r.fIP}
}

func (st *store) anomalyCount() (int, error) { return st.number("select count(*) from x.anomaly") }

func (st *store) anomalies(lo, hi int) ([]anomalyLine, error) {
	return all[anomalyLine](st.db, `select a.rank, a.player_id, p.name, k.rank, k.display, a.k, a.m_gap, a.m_on,
		a.m_ip, a.f_gap, a.f_on, a.f_ip from x.anomaly a join x.player p on p.id = a.player_id
		left join x.player_rank k on k.player_id = a.player_id
		where a.rank > ? and a.rank <= ? order by a.rank`, lo, hi)
}

// found is a search hit: a player, or a team with its town.
type found struct {
	id          int64
	name, town  *string
	rank, games *int64
	display     *float64
	last        *string
}

func (r *found) fields() []any {
	return []any{&r.id, &r.name, &r.town, &r.rank, &r.display, &r.games, &r.last}
}

// searchLimit caps the hits of each kind.
const searchLimit = 40

func (st *store) findPlayers(query string) ([]found, error) {
	return all[found](st.db, `select f.rowid, p.name, null, k.rank, k.display, p.games, p.last_date
		from x.player_fts f join x.player p on p.id = f.rowid
		left join x.player_rank k on k.player_id = f.rowid
		where player_fts match ? order by k.rank is null, k.rank, p.games desc limit ?`, query, searchLimit)
}

func (st *store) findTeams(query string) ([]found, error) {
	return all[found](st.db, `select f.rowid, t.name, t.town, k.rank, k.display, t.games, t.last_date
		from x.team_fts f join x.team t on t.id = f.rowid
		left join x.team_rank k on k.team_id = f.rowid
		where team_fts match ? order by k.rank is null, k.rank, t.games desc limit ?`, query, searchLimit)
}

// The entity pages: a player, a team, their histories and results.

// standing is an entity's last published rating and its place in the
// current table. A rating it never had is nil, and so is a place.
type standing struct {
	name, town, last *string
	games, games365  *int64
	display, sigma   *float64
	prov             *bool
	rank             *int64
	change           *float64
}

func (r *standing) fields() []any {
	return []any{&r.name, &r.town, &r.last, &r.games, &r.games365, &r.display, &r.sigma, &r.prov, &r.rank, &r.change}
}

// rated is whether the entity has a published rating at all.
func (r standing) rated() bool { return r.display != nil }

// provisional is a rating too uncertain to rank, or no rating.
func (r standing) provisional() bool { return r.prov == nil || *r.prov }

func (st *store) player(id int64) (standing, bool, error) {
	return one[standing](st.db, `select p.name, null, p.last_date, p.games, p.games_365, p.display, p.sigma,
		p.provisional, k.rank, k.change from x.player p left join x.player_rank k on k.player_id = p.id
		where p.id = ?`, id)
}

func (st *store) team(id int64) (standing, bool, error) {
	return one[standing](st.db, `select t.name, t.town, t.last_date, t.games, null, t.display, t.sigma,
		t.provisional, k.rank, k.change from x.team t left join x.team_rank k on k.team_id = t.id
		where t.id = ?`, id)
}

// ratingAt is an entity's published rating at one release, with its
// plus-minus on the display scale.
type ratingAt struct {
	rel        int64
	mid, sigma float64
}

type rawPt struct {
	rel          int64
	value, sigma *float64
}

func (r *rawPt) fields() []any { return []any{&r.rel, &r.value, &r.sigma} }

// history is an entity's published rating at each release it was listed in.
func (st *store) history(kind int, id int64) ([]ratingAt, error) {
	query := "select release_id, theta, sigma from player_ratings where player_id = ?"
	a, b := st.sc.aPlayer, st.sc.bPlayer
	if kind == kindTeam {
		query = "select release_id, mu, sigma from team_ratings where team_id = ?"
		a, b = st.sc.aTeam, st.sc.bTeam
	}
	rows, err := all[rawPt](st.db, query, id)
	out := make([]ratingAt, len(rows))
	for i, r := range rows {
		out[i] = ratingAt{r.rel, a + b*num(r.value), abs(b) * num(r.sigma)}
	}
	return out, err
}

// places is where the entity stood in the published table at each of those
// releases, stored as one little-endian u32 blob per entity in the same order.
func (st *store) places(kind int, id int64) ([]uint32, error) {
	var blob []byte
	err := st.db.QueryRow("select places from x.place_hist where kind = ? and id = ?", kind, id).Scan(&blob)
	out := make([]uint32, len(blob)/4)
	for i := range out {
		c := blob[4*i:]
		out[i] = uint32(c[0]) | uint32(c[1])<<8 | uint32(c[2])<<16 | uint32(c[3])<<24
	}
	return out, ignoreNoRows(err)
}

// game is one tournament a player played, with the player's number before
// and after it.
type game struct {
	tid, team                     int64
	tname, date, typ, mname       *string
	position                      *float64
	score, nQ, rated              *int64
	theta, after, phi             *float64
	before, afterShown            *float64
}

func (r *game) fields() []any {
	return []any{&r.tid, &r.tname, &r.date, &r.typ, &r.team, &r.mname, &r.position, &r.score, &r.nQ,
		&r.theta, &r.after, &r.phi, &r.rated}
}

// games are every tournament the player played, newest first. The number
// before and after is the one at an in-person table, like the release
// number and the tournament roster.
func (st *store) games(id int64) ([]game, error) {
	rows, err := all[game](st.db, `select g.tournament_id, t.name, t.date, t.type, g.team_id, m.name, g.position,
		g.score, t.n_q, v.theta_before, v.theta_after, v.phi_before, te.rated
		from x.player_game g join x.tournament t on t.id = g.tournament_id
		left join x.team m on m.id = g.team_id
		left join player_events v on v.player_id = g.player_id and v.tournament_id = g.tournament_id
		left join team_events te on te.tournament_id = g.tournament_id and te.team_id = g.team_id
		where g.player_id = ? order by t.date desc, t.id desc`, id)
	for i := range rows {
		g := &rows[i]
		at := func(x float64) float64 { return st.sc.player(x, num(g.phi)) }
		g.before, g.afterShown = mapped(g.theta, at), mapped(g.after, at)
	}
	return rows, err
}

type rosterLine struct {
	id      int64
	name    *string
	display *float64
	prov    bool
}

func (r *rosterLine) fields() []any { return []any{&r.id, &r.name, &r.display, &r.prov} }

// baseRoster is the team's declared roster, strongest first; a player with
// no current rating goes last.
func (st *store) baseRoster(id int64) ([]rosterLine, error) {
	return all[rosterLine](st.db, `select b.player_id, b.name, k.display, coalesce(k.provisional, 0) from x.base_roster b
		left join x.player_rank k on k.player_id = b.player_id
		where b.team_id = ? order by k.display is null, k.display desc, b.name`, id)
}

// The achievement rating, when the fit published it.

type achieved struct {
	rating, divisor float64
	nResults        int64
}

func (r *achieved) fields() []any { return []any{&r.rating, &r.divisor, &r.nResults} }

func (st *store) achieved(id int64) (achieved, bool, error) {
	if !st.achieve {
		return achieved{}, false, nil
	}
	return one[achieved](st.db, "select rating, divisor, n_results from achieve_team where team_id = ?", id)
}

type rule struct{ key, value string }

func (r *rule) fields() []any { return []any{&r.key, &r.value} }

// achieveRules are the knobs the achievement rating was published with.
func (st *store) achieveRules() (map[string]string, error) {
	out := map[string]string{}
	if !st.achieve {
		return out, nil
	}
	rows, err := all[rule](st.db, "select key, value from achieve_meta")
	for _, r := range rows {
		out[r.key] = r.value
	}
	return out, err
}

// part is one result a team's achievement rating is the sum of.
type part struct {
	tid, n, nv                                                       int64
	name, date, format                                               string
	placeAll, placeOwn, strength, wfmt, wfield, wreach, peak, fade, ov, contrib float64
}

func (r *part) fields() []any {
	return []any{&r.tid, &r.name, &r.date, &r.placeAll, &r.placeOwn, &r.n, &r.nv, &r.strength, &r.wfmt,
		&r.wfield, &r.wreach, &r.peak, &r.fade, &r.ov, &r.contrib, &r.format}
}

func (st *store) parts(id int64) ([]part, error) {
	return all[part](st.db, `select p.tournament_id, coalesce(t.name, ''), a.date, a.place, a.venue_place, a.n_teams,
		a.n_venues, a.strength, a.w_format, a.w_field, a.w_reach, p.peak, p.fade, p.overlap,
		p.contribution, a.format
		from achieve_parts p join achieve_values a on a.tournament_id = p.tournament_id and a.team_id = p.team_id
		left join x.tournament t on t.id = p.tournament_id
		where p.team_id = ? order by p.slot`, id)
}

// result is one tournament a team played, with what it was worth.
type result struct {
	tid                         int64
	name, date, typ             *string
	actual                      *float64
	rated                       *int64
	peak                        *float64
	nTeams                      *int64
	gain, strength, w, placeVal *float64
	counts                      bool
}

func (r *result) fields() []any {
	return []any{&r.tid, &r.name, &r.date, &r.actual, &r.rated, &r.typ, &r.peak, &r.nTeams, &r.gain, &r.strength,
		&r.w, &r.placeVal, &r.counts}
}

// results are every result the team has played, newest first.
func (st *store) results(id int64) ([]result, error) {
	sel, join := "null, null, null, null, null, null, 0", ""
	if st.achieve {
		sel = `a.peak, a.n_teams, a.gain, a.strength, a.w, a.place,
			exists(select 1 from achieve_parts p where p.team_id = v.team_id and p.tournament_id = v.tournament_id)`
		join = "left join achieve_values a on a.tournament_id = v.tournament_id and a.team_id = v.team_id"
	}
	return all[result](st.db, `select v.tournament_id, t.name, coalesce(v.date, t.date) as d, v.actual_place,
		v.rated, t.type, `+sel+`
		from team_events v left join x.tournament t on t.id = v.tournament_id `+join+`
		where v.team_id = ? order by d desc, v.tournament_id desc`, id)
}

// The tournament page.

type tourHead struct {
	name, date, typ                       *string
	weight, llIn, llCf, spF, spO, truedl *float64
	nTeams, nQ, nObs                      *int64
}

func (r *tourHead) fields() []any {
	return []any{&r.name, &r.date, &r.typ, &r.weight, &r.nTeams, &r.nQ, &r.nObs, &r.llIn, &r.llCf, &r.spF, &r.spO, &r.truedl}
}

func (st *store) tournament(id int64) (tourHead, bool, error) {
	return one[tourHead](st.db, `select name, date, type, weight, n_teams, n_q, n_obs, ll_in, ll_cf, sp_forecast,
		sp_old, truedl from x.tournament where id = ?`, id)
}

type venue struct {
	id   int64
	name string
}

func (r *venue) fields() []any { return []any{&r.id, &r.name} }

// venues are the tournaments one package was played as in other cities,
// this one among them; none when it was played in one place.
func (st *store) venues(id int64) ([]venue, error) {
	return all[venue](st.db, `select v.tournament_id, coalesce(t.name, '') from x.venue v
		left join x.tournament t on t.id = v.tournament_id
		where v.field_id = (select field_id from x.venue where tournament_id = ?)
		order by v.tournament_id`, id)
}

// idList is ids straight from the index, as a literal list for the SQL.
func idList(ids []int64) string {
	parts := make([]string, len(ids))
	for i, v := range ids {
		parts[i] = itoa(v)
	}
	return strings.Join(parts, ",")
}

// fieldTeams is how many teams played the package across its venues.
func (st *store) fieldTeams(ids []int64) (*int64, error) {
	var sum *int64
	err := st.db.QueryRow("select sum(n_teams) from x.tournament where id in (" + idList(ids) + ")").Scan(&sum)
	return sum, err
}

type question struct {
	n int64
	b *float64
}

func (r *question) fields() []any { return []any{&r.n, &r.b} }

func (st *store) questions(id int64) ([]question, error) {
	return all[question](st.db, "select q, b from question_params where tournament_id = ? order by q", id)
}

// fieldRow is one team's line in a tournament's field.
type fieldRow struct {
	team                   int64
	name, playedAs         string
	forecast, actual       *float64
	before, after          *float64
	rated                  bool
	score                  *int64
	lineup, dPlace, dPull  *float64
	value, gain            *float64
	venue                  int64
	strength               *float64
	allActual, allForecast *float64
	// step is the whole move of the rating, in display points.
	step *float64
}

type fieldScan struct {
	r                fieldRow
	name, playedAs   *string
	rated            *int64
}

func (f *fieldScan) fields() []any {
	r := &f.r
	return []any{&r.team, &f.name, &r.forecast, &r.actual, &r.before, &r.after, &f.rated, &r.score, &f.playedAs,
		&r.lineup, &r.dPlace, &r.dPull, &r.value, &r.gain, &r.venue, &r.strength}
}

// field is every team at the venues, in the order of its own venue's places,
// with each number on the published scale.
func (st *store) field(ids []int64) ([]fieldRow, error) {
	steps, values, join := "null, null", "null, null", ""
	if st.teamSteps {
		steps = "v.d_place, v.d_pull"
	}
	if st.achieve {
		values = "a.peak, a.gain"
		join = "left join achieve_values a on a.tournament_id = v.tournament_id and a.team_id = v.team_id"
	}
	rows, err := all[fieldScan](st.db, `select v.team_id, m.name, v.forecast_place, v.actual_place, v.mu_before, v.mu_after,
		v.rated, r.score, r.name, v.s_r, `+steps+`, `+values+`, v.tournament_id, v.forecast
		from team_events v left join x.team m on m.id = v.team_id
		left join x.result_ r on r.tournament_id = v.tournament_id and r.team_id = v.team_id `+join+`
		where v.tournament_id in (`+idList(ids)+`) order by v.actual_place`)
	out := make([]fieldRow, len(rows))
	for i, f := range rows {
		out[i] = st.shown(f)
	}
	return out, err
}

// shown puts a field row's rating numbers on the published scale.
func (st *store) shown(f fieldScan) fieldRow {
	r, b := f.r, st.sc.bTeam
	r.name, r.playedAs = str(f.name), str(f.playedAs)
	r.rated = f.rated == nil || *f.rated != 0
	if r.before != nil && r.after != nil {
		step := b * (*r.after - *r.before)
		r.step = &step
	}
	r.before, r.after = mapped(r.before, st.sc.team), mapped(r.after, st.sc.team)
	r.lineup = mapped(r.lineup, st.sc.team)
	times := func(x float64) float64 { return b * x }
	r.dPlace, r.dPull = mapped(r.dPlace, times), mapped(r.dPull, times)
	return r
}

type member struct {
	player        int64
	name          string
	before, after *float64
	provisional   bool
}

type memberScan struct {
	team, tid                int64
	m                        member
	name                     *string
	theta, after, phi, sigma *float64
}

func (r *memberScan) fields() []any {
	return []any{&r.team, &r.m.player, &r.name, &r.theta, &r.after, &r.phi, &r.sigma, &r.tid}
}

// lineups are the players at each table, keyed by (venue, team), strongest
// first, with each player's published number on the morning of the game.
func (st *store) lineups(ids []int64) (map[[2]int64][]member, error) {
	rows, err := all[memberScan](st.db, `select g.team_id, g.player_id, p.name, v.theta_before, v.theta_after, v.phi_before,
		v.sigma_before, g.tournament_id from x.player_game g left join x.player p on p.id = g.player_id
		left join player_events v on v.player_id = g.player_id and v.tournament_id = g.tournament_id
		where g.tournament_id in (` + idList(ids) + `)`)
	out := map[[2]int64][]member{}
	for _, r := range rows {
		k := [2]int64{r.tid, r.team}
		out[k] = append(out[k], st.member(r))
	}
	for _, v := range out {
		sort.SliceStable(v, func(i, j int) bool { return stronger(v[i], v[j]) })
	}
	return out, err
}

// member is a lineup's player with the number at an in-person table, which
// is what the roster lists.
func (st *store) member(r memberScan) member {
	m := r.m
	m.name = str(r.name)
	m.provisional = r.sigma != nil && *r.sigma > st.sc.provSigma
	at := func(x float64) float64 { return st.sc.player(x, num(r.phi)) }
	m.before, m.after = mapped(r.theta, at), mapped(r.after, at)
	return m
}

// stronger orders a lineup: the higher number first, a player with none
// last, then by name.
func stronger(a, b member) bool {
	x, y := orLowest(a.before), orLowest(b.before)
	if x != y {
		return x > y
	}
	return a.name < b.name
}

// orLowest is a nullable number, or minus infinity for none.
func orLowest(p *float64) float64 {
	if p == nil {
		return math.Inf(-1)
	}
	return *p
}

// ignoreNoRows is err, except that a row not there is not an error.
func ignoreNoRows(err error) error {
	if errors.Is(err, sql.ErrNoRows) {
		return nil
	}
	return err
}
