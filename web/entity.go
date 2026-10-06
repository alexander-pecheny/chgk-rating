package main

import (
	"database/sql"
	"errors"
	"fmt"
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
func (s *site) history(sqlText string, id int64, a, b float64) []histPt {
	rows, err := s.db.Query(sqlText, id)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var v []histPt
	for rows.Next() {
		var rel int64
		var value, sigma *float64
		if rows.Scan(&rel, &value, &sigma) != nil {
			continue
		}
		i, ok := s.relIdx[rel]
		if !ok {
			continue
		}
		v = append(v, histPt{i, a + b*num(value), abs(b) * num(sigma)})
	}
	sort.SliceStable(v, func(i, j int) bool { return v[i].rel < v[j].rel })
	return v
}

// places is where the entity stood in the published table at each of those
// releases, stored as one little-endian u32 blob per entity in the same order.
func (s *site) places(kind int, id int64) []uint32 {
	var blob []byte
	if s.db.QueryRow("select places from x.place_hist where kind = ? and id = ?", kind, id).Scan(&blob) != nil {
		return nil
	}
	out := make([]uint32, len(blob)/4)
	for i := range out {
		c := blob[4*i:]
		out[i] = uint32(c[0]) | uint32(c[1])<<8 | uint32(c[2])<<16 | uint32(c[3])<<24
	}
	return out
}

// chartSection is the rating history and, beside it, the same weeks as a place
// in the table.
func (s *site) chartSection(kind int, id int64, sqlText string, a, b float64, band bool) ui.Node {
	hist := s.history(sqlText, id, a, b)
	if len(hist) < 2 {
		return nil
	}
	pl := s.places(kind, id)
	rating := make([]pt, len(hist))
	for i, h := range hist {
		text := disp(h.mid)
		if band {
			text = disp(h.mid) + " ± " + disp(h.sigma)
		}
		rating[i] = pt{x: float64(h.rel), mid: h.mid, sigma: h.sigma, date: s.rels[h.rel].date, text: text}
	}
	// Lengths differ only if the index was built against another ratings file;
	// taking the shorter of the two keeps the weeks lined up either way.
	var placePts []pt
	for i, h := range hist {
		if i >= len(pl) {
			break
		}
		n := pl[i]
		placePts = append(placePts, pt{x: float64(h.rel), mid: float64(n), date: s.rels[h.rel].date, text: "№" + strconv.Itoa(int(n))})
	}
	from, to := hist[0].rel, hist[len(hist)-1].rel
	ss := []series{
		{name: S.Common.Chart.Rating(), pts: thin(rating, chartPoints), band: band},
		{name: S.Common.Chart.Place(), pts: thin(placePts, chartPoints), invert: true},
	}
	return ui.Section(ui.Subhead(ui.Text(S.Common.Chart.Title())), chart(ss, s.ticks(from, to), S.Common.Chart.Title()))
}

func (s *site) playerPage(id int64) (string, []ui.Item, error) {
	var name, last *string
	var games *int64
	err := s.db.QueryRow("select name, games, last_date from x.player where id = ?", id).Scan(&name, &games, &last)
	if errors.Is(err, sql.ErrNoRows) {
		return "", nil, errNotFound
	} else if err != nil {
		return "", nil, err
	}
	nm := str(name)
	if nm == "" {
		nm = S.Common.PlayerFallback(itoa(id))
	}

	// The last release this player was listed in, which for anyone still active
	// is the current one and for everyone else is where their history stops.
	var display, sigma *float64
	var g365 *int64
	hasNow := s.db.QueryRow(`select display, sigma, games_365 from player_ratings where player_id = ?
		order by release_id desc limit 1`, id).Scan(&display, &sigma, &g365) == nil
	var rank int64
	var change *float64
	listed := s.db.QueryRow("select rank, change from x.player_rank where player_id = ?", id).Scan(&rank, &change) == nil
	provisional := !hasNow || num(sigma) > s.d.provSigma

	byline := []ui.Item{ui.Text(S.Player.NotListed(str(last)))}
	if listed {
		byline = []ui.Item{ui.Text(S.Player.Rank(itoa(rank)))}
		if provisional {
			byline = append(byline, ui.Text(" "), provBadge())
		}
	}
	stats := []ui.Item{}
	if hasNow {
		stats = append(stats,
			stat(S.Common.Col.Rating(), disp(num(display))),
			stat(S.Player.Stat.Sigma(), "±"+disp(s.d.bPlayer*num(sigma))),
			stat(S.Player.Stat.Games365(), itoa(count(g365))),
		)
	} else {
		stats = append(stats, stat(S.Common.Col.Rating(), "—"))
	}
	stats = append(stats,
		stat(S.Player.Stat.Games(), itoa(count(games))),
		stat(S.Player.Stat.Week(), signedOpt(change)),
	)
	out := []ui.Item{
		ui.Pagehead(ui.Byline(ui.Inline(byline...))),
		ui.Stats(stats...),
	}
	if c := s.chartSection(0, id, "select release_id, theta, sigma from player_ratings where player_id = ?", s.d.aPlayer, s.d.bPlayer, true); c != nil {
		out = append(out, c)
	}

	// Before and after at an in-person table, theta + phi, like the release
	// number and the tournament roster.
	rows, err := s.db.Query(`select g.tournament_id, t.name, t.date, t.type, g.team_id, m.name, g.position,
		g.score, t.n_q, v.theta_before + coalesce(v.phi_before, 0),
		v.theta_after + coalesce(v.phi_before, 0), te.rated
		from x.player_game g join x.tournament t on t.id = g.tournament_id
		left join x.team m on m.id = g.team_id
		left join player_events v on v.player_id = g.player_id and v.tournament_id = g.tournament_id
		left join team_events te on te.tournament_id = g.tournament_id and te.team_id = g.team_id
		where g.player_id = ? order by t.date desc, t.id desc`, id)
	if err != nil {
		return "", nil, err
	}
	defer rows.Close()
	table := []ui.Item{ui.Scroll(), ui.Trow(
		hcell(S.Common.Col.Date()),
		hcell(S.Common.Col.Tournament(), ui.PinCol()),
		hcell(S.Common.Col.Type()),
		hcell(S.Common.Col.Team()),
		hcell(S.Common.Col.Place(), ui.Num()),
		hcell(S.Common.Col.Taken(), ui.Num()),
		hcell(S.Common.Col.Before(), ui.Num()),
		hcell(S.Common.Col.After(), ui.Num()),
		hcell("Δ", ui.Num()),
	)}
	n := 0
	for rows.Next() {
		var tid, team int64
		var tname, date, typ, mname *string
		var position, before, after *float64
		var score, nQ, rated *int64
		if err := rows.Scan(&tid, &tname, &date, &typ, &team, &mname, &position, &score, &nQ, &before, &after, &rated); err != nil {
			return "", nil, err
		}
		n++
		taken := "—"
		if score != nil {
			taken = itoa(*score)
			if count(nQ) > 0 {
				taken += "/" + itoa(*nQ)
			}
		}
		cells := []ui.Item{
			textCell(str(date), ui.NoWrap()),
			nameCell(fmt.Sprintf("/tournaments/%d/", tid), orUntitled(str(tname)), unratedFlag(rated == nil || *rated != 0)),
			textCell(tourType(str(typ)), ui.Faint()),
			ui.Cell(line(link(fmt.Sprintf("/teams/%d/", team), orDash(str(mname))))),
			numCell(place(position)),
			numCell(taken),
		}
		cells = append(cells, stepCells(s.d.aPlayer, s.d.bPlayer, before, after)...)
		table = append(table, ui.Trow(cells...))
	}
	if err := rows.Err(); err != nil {
		return "", nil, err
	}
	games_ := []ui.Item{ui.Subhead(ui.Text(S.Player.Games(itoa(int64(n)))))}
	if n == 0 {
		games_ = append(games_, ui.Empty(ui.Text(S.Common.NoTournaments())))
	} else {
		games_ = append(games_, ui.Table(table...))
	}
	return nm, append(out, ui.Section(games_...)), nil
}

func (s *site) teamPage(id int64) (string, []ui.Item, error) {
	var name, town *string
	err := s.db.QueryRow("select name, town from x.team where id = ?", id).Scan(&name, &town)
	if errors.Is(err, sql.ErrNoRows) {
		return "", nil, errNotFound
	} else if err != nil {
		return "", nil, err
	}
	nm := str(name)
	if nm == "" {
		nm = S.Common.TeamFallback(itoa(id))
	}
	townText := str(town)
	if townText == "" {
		townText = S.Team.NoTown()
	}

	var display, sigma *float64
	hasNow := s.db.QueryRow(`select display, sigma from team_ratings
		where team_id = ? order by release_id desc limit 1`, id).Scan(&display, &sigma) == nil
	var rank int64
	var change *float64
	listed := s.db.QueryRow("select rank, change from x.team_rank where team_id = ?", id).Scan(&rank, &change) == nil
	provisional := !hasNow || num(sigma) > s.d.provSigma

	byline := []ui.Item{ui.Text(S.Team.NotListed(townText))}
	if listed {
		byline = []ui.Item{ui.Text(S.Team.Rank(itoa(rank), townText))}
		if provisional {
			byline = append(byline, ui.Text(" "), provBadge())
		}
	}

	// The achievement rating, when the database carries it.
	rules := s.achieveRules()
	var rating, divisor float64
	var nResults int64
	hasAchieve := s.db.QueryRow("select rating, divisor, n_results from achieve_team where team_id = ?", id).Scan(&rating, &divisor, &nResults) == nil

	ratingText := "—"
	if hasNow {
		ratingText = disp(num(display))
	}
	placeText := "—"
	if listed {
		placeText = "№" + itoa(rank)
	}
	stats := []ui.Item{
		stat(S.Common.Col.Rating(), ratingText),
		stat(S.Common.Col.Place(), placeText),
		stat(S.Team.Stat.Week(), signedOpt(change)),
	}
	if x, ok := rules["top_x"]; ok && hasAchieve {
		stats = append(stats, stat(S.Team.Stat.Results(), S.Team.Stat.ResultsOf(itoa(nResults), x)))
	}
	out := []ui.Item{
		ui.Pagehead(ui.Byline(ui.Inline(byline...))),
		ui.Stats(stats...),
	}
	if hasAchieve {
		b, err := s.achieveBreakdown(id, rating, divisor, rules)
		if err != nil {
			return "", nil, err
		}
		out = append(out, b)
	}
	if c := s.chartSection(1, id, "select release_id, mu, sigma from team_ratings where team_id = ?", s.d.aTeam, s.d.bTeam, false); c != nil {
		out = append(out, c)
	}

	// The base roster in force at the last release, strongest first; a player
	// with no current rating goes last.
	if roster := s.baseRoster(id); roster != nil {
		out = append(out, roster)
	}

	results, err := s.teamResults(id)
	if err != nil {
		return "", nil, err
	}
	return nm, append(out, results), nil
}

func (s *site) baseRoster(id int64) ui.Node {
	rows, err := s.db.Query(`select b.player_id, b.name, k.display, coalesce(k.provisional, 0) from x.base_roster b
		left join x.player_rank k on k.player_id = b.player_id
		where b.team_id = ? order by k.display is null, k.display desc, b.name`, id)
	if err != nil {
		return nil
	}
	defer rows.Close()
	var members []ui.Item
	for rows.Next() {
		var pid int64
		var pname *string
		var display *float64
		var prov bool
		if rows.Scan(&pid, &pname, &display, &prov) != nil {
			continue
		}
		var badge ui.Item
		if prov {
			badge = provBadge()
		}
		members = append(members, ui.Member(line(
			link(fmt.Sprintf("/players/%d/", pid), str(pname)), space(badge), badge,
			ui.Text(" "), ui.Fig(ui.Text(dispOpt(display))),
		)))
	}
	if len(members) == 0 {
		return nil
	}
	return ui.Section(ui.Subhead(ui.Text(S.Team.Roster(itoa(int64(len(members)))))), ui.Roster(members...))
}

// achieveRules are the knobs the achievement rating was published with.
func (s *site) achieveRules() map[string]string {
	out := map[string]string{}
	rows, err := s.db.Query("select key, value from achieve_meta")
	if err != nil {
		return out
	}
	defer rows.Close()
	for rows.Next() {
		var k, v string
		if rows.Scan(&k, &v) == nil {
			out[k] = v
		}
	}
	return out
}

// rulePct is a share from the rules, formatted for a sentence: 0.1 is "10%".
func rulePct(rules map[string]string, key string) string {
	v, err := strconv.ParseFloat(rules[key], 64)
	if err != nil {
		return "?"
	}
	return pct(v) + "%"
}

// achieveBreakdown is the rating taken apart: the results it is the sum of,
// each with what it was worth on the day, how much of that is left, and what
// it adds.
func (s *site) achieveBreakdown(id int64, rating, divisor float64, rules map[string]string) (ui.Node, error) {
	rows, err := s.db.Query(`select p.tournament_id, coalesce(t.name, ''), a.date, a.place, a.venue_place, a.n_teams,
		a.n_venues, a.strength, a.w_format, a.w_field, a.w_reach, p.peak, p.fade, p.overlap,
		p.contribution, a.format
		from achieve_parts p join achieve_values a on a.tournament_id = p.tournament_id and a.team_id = p.team_id
		left join x.tournament t on t.id = p.tournament_id
		where p.team_id = ? order by p.slot`, id)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	x, ok := rules["top_x"]
	if !ok {
		x = fmt.Sprintf("%.0f", divisor)
	}
	pow, ok := rules["place_pow"]
	if !ok {
		pow = "0.6"
	}
	table := []ui.Item{ui.Scroll(), ui.Breakdown(), ui.Trow(
		hcell(S.Common.Col.Tournament(), ui.PinCol()),
		hcell(S.Common.Col.Place(), ui.Num()),
		hcell(S.Common.Col.Value(), ui.Num(), ui.Title(S.Common.Col.ValueTitle())),
		hcell(S.Team.Breakdown.Left(), ui.Num(), ui.Title(S.Team.Breakdown.LeftTitle())),
		hcell(S.Team.Breakdown.Now(), ui.Num(), ui.Title(S.Team.Breakdown.NowTitle())),
	)}
	sum := 0.0
	parts := 0
	for rows.Next() {
		var tid, n, nv int64
		var name, date, format string
		var placeAll, placeOwn, strength, wfmt, wfield, wreach, peak, fade, ov, contrib float64
		if err := rows.Scan(&tid, &name, &date, &placeAll, &placeOwn, &n, &nv, &strength, &wfmt, &wfield, &wreach, &peak, &fade, &ov, &contrib, &format); err != nil {
			return nil, err
		}
		parts++
		sum += contrib
		// How the value on the day was made, in one line under the name.
		how := []ui.Item{ui.Text(S.Team.Breakdown.Game(disp(strength)))}
		if placeAll != 1 {
			how = append(how, ui.Text(" "+S.Team.Breakdown.ByPlace()), ui.Sup(ui.Text(pow)))
		}
		w := wfmt * wfield * wreach
		if abs(w-1) >= 0.005 {
			var why []string
			if wfmt < 0.999 {
				f := S.Common.Format.Sync()
				if format == "S" {
					f = S.Common.Format.StrictSync()
				}
				why = append(why, f+" ×"+w2(wfmt))
			}
			if wfield < 0.995 {
				why = append(why, S.Team.Breakdown.Field(w2(wfield)))
			}
			if wreach < 0.995 {
				why = append(why, S.Team.Breakdown.Reach(w2(wreach)))
			}
			how = append(how, ui.Text(" "+S.Team.Breakdown.Weight(w2(w), strings.Join(why, ", "))))
		}
		if len(how) > 1 {
			how = append(how, ui.Text(" = "+disp(peak)))
		}
		leftTitle := S.Team.Breakdown.Fade(pct(fade))
		if ov < 0.995 {
			leftTitle = S.Team.Breakdown.FadeOverlap(pct(fade), pct(ov))
		}
		// One run of text: the line under a name is a flex row, and two text
		// pieces side by side in it would lose the space between them.
		under := date
		if nv > 1 {
			under += " · " + S.Team.Breakdown.Merged(int(nv), placeOf(placeOwn))
		}
		table = append(table, ui.Trow(
			ui.Cell(ui.PinCol(), line(link(fmt.Sprintf("/tournaments/%d/", tid), orUntitled(name))), subline(ui.Text(under)), ui.How(ui.Inline(how...))),
			ui.Cell(ui.Num(), ui.NoWrap(), ui.Inline(ui.Text(placeOf(placeAll)+" "), ui.Muted(ui.Text(S.Common.OfN(itoa(n)))))),
			numCell(disp(peak)),
			numCell(pct(fade*ov)+"%", ui.Faint(), ui.Title(leftTitle)),
			numCell(disp(contrib*divisor), ui.Bold()),
		))
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	slots, err := strconv.Atoi(x)
	if err != nil {
		slots = parts
	}
	for i := parts; i < slots; i++ {
		table = append(table, ui.Trow(
			textCell(S.Team.Breakdown.EmptySlot(), ui.Faint()), emptyCell(), emptyCell(), emptyCell(),
			numCell("0", ui.Faint()),
		))
	}
	table = append(table, ui.Trow(ui.FootRow(),
		textCell(S.Team.Breakdown.Sum(disp(sum*divisor), fmt.Sprintf("%.0f", divisor)), ui.Colspan("4")),
		numCell(disp(rating), ui.Bold()),
	))
	return ui.Section(
		ui.Subhead(ui.Text(S.Team.Breakdown.Title())),
		ui.Hint(ui.Inline(
			ui.Text(S.Team.Breakdown.HowLead(x)), ui.Sup(ui.Text(pow)),
			ui.Text(S.Team.Breakdown.HowTail(rulePct(rules, "loss_ip"), rulePct(rules, "loss_sync"))+" "),
			link("/method/", S.Team.Breakdown.HowLink()), ui.Text("."),
		)),
		ui.Table(table...),
	), nil
}

// teamResults is every result the team has played, with what it was worth and
// whether it counts today.
func (s *site) teamResults(id int64) (ui.Node, error) {
	haveValues := tableExists(s.db, "achieve_values")
	sel := "null, null, null, null, null, null, 0"
	join := ""
	if haveValues {
		sel = `a.peak, a.n_teams, a.gain, a.strength, a.w, a.place,
			exists(select 1 from achieve_parts p where p.team_id = v.team_id and p.tournament_id = v.tournament_id)`
		join = "left join achieve_values a on a.tournament_id = v.tournament_id and a.team_id = v.team_id"
	}
	rows, err := s.db.Query(`select v.tournament_id, t.name, coalesce(v.date, t.date) as d, v.actual_place,
		v.rated, t.type, `+sel+`
		from team_events v left join x.tournament t on t.id = v.tournament_id `+join+`
		where v.team_id = ? order by d desc, v.tournament_id desc`, id)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	type ev struct {
		tid                         int64
		name, date, typ             *string
		actual                      *float64
		rated                       *int64
		peak                        *float64
		nTeams                      *int64
		gain, strength, w, placeVal *float64
		counts                      bool
	}
	var evs []ev
	for rows.Next() {
		var e ev
		if err := rows.Scan(&e.tid, &e.name, &e.date, &e.actual, &e.rated, &e.typ, &e.peak, &e.nTeams, &e.gain, &e.strength, &e.w, &e.placeVal, &e.counts); err != nil {
			return nil, err
		}
		evs = append(evs, e)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	section := []ui.Item{ui.Subhead(ui.Text(S.Team.Results.Title(itoa(int64(len(evs))))))}
	if len(evs) == 0 {
		return ui.Section(append(section, ui.Empty(ui.Text(S.Common.NoTournaments())))...), nil
	}
	section = append(section, ui.Hint(ui.Text(S.Team.Results.Hint())))
	// The last three years show; older results open with the switch.
	cutoff := ""
	if y, err := strconv.Atoi(prefix(s.releaseDate, 4)); err == nil && len(s.releaseDate) >= 4 {
		cutoff = strconv.Itoa(y-3) + s.releaseDate[4:]
	}
	older := 0
	for _, e := range evs {
		if str(e.date) < cutoff {
			older++
		}
	}
	if older > 0 {
		section = append(section, ui.Toggle(ui.ToggleOlder, ui.Inline(
			ui.Text(S.Team.Results.OlderToggle()+" "), ui.Count(ui.Text(S.Team.Results.OlderCount(itoa(int64(older)), cutoff))),
		)))
	}
	table := []ui.Item{ui.Scroll(), ui.Trow(
		hcell(S.Common.Col.Date()),
		hcell(S.Common.Col.Tournament(), ui.PinCol()),
		hcell(S.Common.Col.Type()),
		hcell(S.Common.Col.Place(), ui.Num()),
		hcell(S.Common.Col.Value(), ui.Num(), ui.Title(S.Common.Col.ValueTitle())),
		hcell(S.Team.Results.Counts(), ui.Num(), ui.Title(S.Team.Results.CountsColTitle())),
		hcell(S.Common.Col.Gain(), ui.Num(), ui.Title(S.Common.Col.GainTitle())),
	)}
	for _, e := range evs {
		placeCell := []ui.Item{ui.Num(), ui.NoWrap(), ui.Text(place(e.actual))}
		if e.nTeams != nil {
			placeCell = []ui.Item{ui.Num(), ui.NoWrap(), ui.Inline(ui.Text(place(e.actual)+" "), ui.Muted(ui.Text(S.Common.OfN(itoa(*e.nTeams)))))}
		}
		value := []ui.Item{ui.Num(), ui.Text(dispOpt(e.peak))}
		if e.strength != nil && e.w != nil && e.placeVal != nil {
			t := S.Team.Results.ValueHow(disp(*e.strength), w2(*e.w))
			if *e.placeVal != 1 {
				t = S.Team.Results.ValueHowPlace(disp(*e.strength), placeOf(*e.placeVal), w2(*e.w))
			}
			value = append(value, ui.Title(t))
		}
		in := emptyCell(ui.Num())
		if e.counts {
			in = ui.Cell(ui.Num(), ui.Inline(ui.Dotin()))
		}
		row := []ui.Item{
			textCell(str(e.date), ui.NoWrap()),
			nameCell(fmt.Sprintf("/tournaments/%d/", e.tid), orUntitled(str(e.name)), unratedFlag(e.rated == nil || *e.rated != 0)),
			textCell(tourType(str(e.typ)), ui.Faint()),
			ui.Cell(placeCell...),
			ui.Cell(value...),
			in,
			gainCell(e.gain),
		}
		switch {
		case e.counts:
			row = append(row, ui.CountsRow())
		case str(e.date) < cutoff:
			row = append(row, ui.OlderRow())
		}
		table = append(table, ui.Trow(row...))
	}
	return ui.Section(append(section, ui.Table(table...))...), nil
}

func prefix(s string, n int) string {
	if len(s) < n {
		return s
	}
	return s[:n]
}

func abs(x float64) float64 {
	if x < 0 {
		return -x
	}
	return x
}
