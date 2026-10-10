package main

import (
	"errors"
	"sort"
	"strconv"
	"strings"

	"ratingweb/ui"
)

// errNotFound is a page whose entity the index does not know.
var errNotFound = errors.New("not found")

type histPt struct {
	rel        int
	mid, sigma float64
}

// history is an entity's published rating at each release, in release order.
func (s *site) history(kind int, id int64) ([]histPt, error) {
	rows, err := s.q.history(kind, id)
	var v []histPt
	for _, r := range rows {
		if i, ok := s.relIdx[r.rel]; ok {
			v = append(v, histPt{i, r.mid, r.sigma})
		}
	}
	sort.SliceStable(v, func(i, j int) bool { return v[i].rel < v[j].rel })
	return v, err
}

// chartSection is the rating history and, beside it, the same weeks as a place
// in the table; nothing for an entity with fewer than two releases.
func (s *site) chartSection(kind int, id int64, band bool, at *release) (ui.Node, error) {
	hist, err := s.history(kind, id)
	hist = s.upTo(hist, at)
	if err != nil || len(hist) < 2 {
		return nil, err
	}
	pl, err := s.q.places(kind, id)
	from, to := hist[0].rel, hist[len(hist)-1].rel
	ss := []series{
		{name: S.Common.Chart.Rating(), pts: thin(s.ratingPts(hist, band), chartPoints), band: band},
		{name: S.Common.Chart.Place(), pts: thin(s.placePts(hist, pl), chartPoints), invert: true},
	}
	ticks := func(marks int) []tick { return s.ticks(from, to, marks) }
	return section(S.Common.Chart.Title(), charts(ss, ticks, S.Common.Chart.Title())...), err
}

func (s *site) ratingPts(hist []histPt, band bool) []pt {
	out := make([]pt, len(hist))
	for i, h := range hist {
		text := disp(h.mid)
		if band {
			text = disp(h.mid) + " ± " + disp(h.sigma)
		}
		out[i] = pt{x: float64(h.rel), mid: h.mid, sigma: h.sigma, date: s.rels[h.rel].date, text: text}
	}
	return out
}

// placePts are the places at the same releases. Lengths differ only if the
// index was built against another ratings file; taking the shorter of the
// two keeps the weeks lined up either way.
func (s *site) placePts(hist []histPt, pl []uint32) []pt {
	var out []pt
	for i, h := range hist[:min(len(hist), len(pl))] {
		n := pl[i]
		out = append(out, pt{x: float64(h.rel), mid: float64(n), date: s.rels[h.rel].date, text: S.Common.Chart.PlacePoint(strconv.Itoa(int(n)))})
	}
	return out
}

// byline is where the entity stands: its place, with the chip when the
// rating is provisional, or the line for one not in the table.
func byline(e standing, ranked func(string) string, unlisted string) ui.Node {
	items := []ui.Item{ui.Text(unlisted)}
	if e.rank != nil {
		items = []ui.Item{ui.Text(ranked(itoa(*e.rank)))}
		if e.provisional() {
			items = append(items, ui.Text(" "), provBadge())
		}
	}
	return ui.Pagehead(ui.Byline(ui.Inline(items...)))
}

func (s *site) playerPage(id int64, at *release) (string, []ui.Item, error) {
	p, ok, err := s.q.player(id)
	if !ok || err != nil {
		return "", nil, orNotFound(err)
	}
	f, hasRow, err := s.q.fitRowAt(id, at)
	if err != nil {
		return "", nil, err
	}
	games, err := s.q.games(id)
	games = before(games, at, func(g game) string { return str(g.date) })
	if err != nil {
		return "", nil, err
	}
	href, unlisted := playerHref(id), S.Player.NotListed(str(p.last))
	if at != nil {
		rank, err := s.rankAt(kindPlayer, id, *at)
		if err != nil {
			return "", nil, err
		}
		p, unlisted = s.playerAt(p, f, hasRow, rank, len(games)), S.Player.NotListedAt(at.date)
	}
	stats := playerStats(p)
	if hasRow {
		stats = append(stats, fitStats(f)...)
	}
	out := []ui.Item{s.asOfHead(href, at), byline(p, S.Player.Rank, unlisted), ui.Stats(stats...)}
	if at == nil {
		out = append(out, s.pastLink(href))
	}
	c, err := s.chartSection(kindPlayer, id, true, at)
	return playerName(p.name, id), append(out, c, gamesSection(games)), err
}

// fitStats are the model's own numbers behind the rating: the strength, its
// uncertainty and the volatility, each on the scale of the formulas.
func fitStats(f fitRow) []ui.Item {
	const digits = 3
	d := func(x float64) string {
		return strings.Replace(strconv.FormatFloat(x, 'f', digits, float64Bits), ".", ",", 1)
	}
	stats := []ui.Item{stat(S.Player.Stat.Theta(), d(f.theta)), stat(S.Player.Stat.SigmaTheta(), d(f.sigma))}
	if f.vol != nil {
		stats = append(stats, stat(S.Player.Stat.Vol(), d(*f.vol)))
	}
	return stats
}

// orNotFound is err, or errNotFound when the query found nothing.
func orNotFound(err error) error {
	if err == nil {
		return errNotFound
	}
	return err
}

func playerStats(p standing) []ui.Item {
	stats := []ui.Item{stat(S.Common.Col.Rating(), dash)}
	switch {
	case p.rated() && p.rank == nil:
		// Not in this week's table: the rating is the one they last had, and
		// a count of games in the past year would be that year's, not this one.
		stats = []ui.Item{stat(S.Player.Stat.LastRating(), disp(num(p.display))), stat(S.Player.Stat.Sigma(), "±"+disp(num(p.sigma)))}
	case p.rated():
		stats = []ui.Item{
			stat(S.Common.Col.Rating(), disp(num(p.display))),
			stat(S.Player.Stat.Sigma(), "±"+disp(num(p.sigma))),
			stat(S.Player.Stat.Games365(), itoa(count(p.games365))),
		}
	}
	return append(stats, stat(S.Player.Stat.Games(), itoa(count(p.games))), stat(S.Player.Stat.Week(), signedOpt(p.change)))
}

// gamesSection is every tournament the player played, with the number before
// and after it.
func gamesSection(games []game) ui.Node {
	title := S.Player.Games(itoa(int64(len(games))))
	if len(games) == 0 {
		return section(title, ui.Empty(ui.Text(S.Common.NoTournaments())))
	}
	rows := make([]ui.Item, len(games))
	for i, g := range games {
		rows[i] = gameRow(g)
	}
	cols := append(tourCols(), hcell(S.Common.Col.Team()), numCol(S.Common.Col.Place()), numCol(S.Common.Col.Taken()))
	return section(title, table(append(cols, stepCols()...), rows...))
}

func gameRow(g game) ui.Node {
	cells := append(tourCells(str(g.date), g.tid, str(g.tname), str(g.typ), counted(g.rated)),
		ui.Cell(line(link(teamHref(g.team), orDash(str(g.mname))))),
		numCell(place(g.position)),
		numCell(taken(g.score, g.nQ)),
	)
	return ui.Trow(append(cells, stepCells(g.before, g.afterShown)...)...)
}

// taken is the questions a team took, out of the package when it is known.
func taken(score, nQ *int64) string {
	if score == nil {
		return dash
	}
	if count(nQ) > 0 {
		return itoa(*score) + "/" + itoa(*nQ)
	}
	return itoa(*score)
}

func (s *site) teamPage(id int64, at *release) (string, []ui.Item, error) {
	t, ok, err := s.q.team(id)
	if !ok || err != nil {
		return "", nil, orNotFound(err)
	}
	href := teamHref(id)
	if at != nil {
		row, hasRow, err := s.q.teamRowAt(id, *at)
		rank, err2 := s.rankAt(kindTeam, id, *at)
		if err = errors.Join(err, err2); err != nil {
			return "", nil, err
		}
		t = teamAt(t, row, hasRow, rank)
		head := []ui.Item{s.asOfHead(href, at), teamByline(t, at), ui.Stats(teamStats(t)...)}
		c, err := s.chartSection(kindTeam, id, false, at)
		results, err2 := s.teamResults(id, at)
		return teamName(t.name, id), append(head, c, results), errors.Join(err, err2)
	}
	rules, err := s.q.achieveRules()
	if err != nil {
		return "", nil, err
	}
	a, hasAchieve, err := s.q.achieved(id)
	if err != nil {
		return "", nil, err
	}
	head := []ui.Item{teamByline(t, nil), ui.Stats(teamStats(t)...), s.pastLink(href)}
	body, err := s.teamBody(id, a, hasAchieve, rules)
	return teamName(t.name, id), append(head, body...), err
}

func teamByline(t standing, at *release) ui.Node {
	town := str(t.town)
	if town == "" {
		town = S.Team.NoTown()
	}
	ranked := func(rank string) string { return S.Team.Rank(rank, town) }
	unlisted := S.Team.NotListed(town)
	if at != nil {
		unlisted = S.Team.NotListedAt(at.date, town)
	}
	return byline(t, ranked, unlisted)
}

func teamStats(t standing) []ui.Item {
	place := dash
	if t.rank != nil {
		place = itoa(*t.rank)
	}
	return []ui.Item{
		stat(S.Common.Col.Rating(), dispOpt(t.display)),
		stat(S.Common.Col.Place(), place),
		stat(S.Team.Stat.Week(), signedOpt(t.change)),
	}
}

// teamBody is the team page under its stats: the achievement breakdown, the
// history, the declared roster and every result.
func (s *site) teamBody(id int64, a achieved, hasAchieve bool, rules map[string]string) ([]ui.Item, error) {
	var out []ui.Item
	if hasAchieve {
		b, err := s.achieveBreakdown(id, a, rules)
		if err != nil {
			return nil, err
		}
		out = append(out, b)
	}
	c, err := s.chartSection(kindTeam, id, false, nil)
	roster, err2 := s.baseRoster(id)
	results, err3 := s.teamResults(id, nil)
	return append(out, c, roster, results), errors.Join(err, err2, err3)
}

// baseRoster is the roster in force at the last release, strongest first; a
// player with no current rating goes last.
func (s *site) baseRoster(id int64) (ui.Node, error) {
	rows, err := s.q.baseRoster(id)
	if len(rows) == 0 {
		return nil, err
	}
	members := make([]ui.Item, len(rows))
	for i, r := range rows {
		members[i] = memberLine(r.id, str(r.name), r.prov, r.display)
	}
	return section(S.Team.Roster(itoa(int64(len(members)))), ui.Roster(members...)), err
}
