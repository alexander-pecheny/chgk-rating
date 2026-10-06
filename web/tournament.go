package main

import (
	"database/sql"
	"errors"
	"fmt"
	"math"
	"sort"
	"strings"

	"ratingweb/ui"
)

type venue struct {
	id   int64
	name string
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
}

type member struct {
	player        int64
	name          string
	before, after *float64
	provisional   bool
}

func (s *site) tournamentPage(id int64) (string, []ui.Item, error) {
	var name, date, typ *string
	var weight, llIn, llCf, spF, spO, truedl *float64
	var nTeams, nQ, nObs *int64
	err := s.db.QueryRow(`select name, date, type, weight, n_teams, n_q, n_obs, ll_in, ll_cf, sp_forecast,
		sp_old, truedl from x.tournament where id = ?`, id).Scan(&name, &date, &typ, &weight, &nTeams, &nQ, &nObs, &llIn, &llCf, &spF, &spO, &truedl)
	if errors.Is(err, sql.ErrNoRows) {
		return "", nil, errNotFound
	} else if err != nil {
		return "", nil, err
	}

	// One package played in several cities is one field: whichever venue the
	// reader opens, the page shows every venue's teams, ranked together.
	var venues []venue
	if rows, err := s.db.Query(`select v.tournament_id, coalesce(t.name, '') from x.venue v
		left join x.tournament t on t.id = v.tournament_id
		where v.field_id = (select field_id from x.venue where tournament_id = ?)
		order by v.tournament_id`, id); err == nil {
		for rows.Next() {
			var v venue
			if rows.Scan(&v.id, &v.name) == nil {
				venues = append(venues, v)
			}
		}
		rows.Close()
	}
	merged := len(venues) > 1
	// Ids straight from the index, so they go into the SQL as a literal list.
	ids := itoa(id)
	if merged {
		parts := make([]string, len(venues))
		for i, v := range venues {
			parts[i] = itoa(v.id)
		}
		ids = strings.Join(parts, ",")
	}
	venueName := func(v int64) string {
		for _, x := range venues {
			if x.id == v {
				return x.name
			}
		}
		return ""
	}

	title := str(name)
	if title == "" {
		title = S.Tournament.Untitled()
	}
	headKids := []ui.Item{
		ui.Byline(ui.Text(S.Tournament.Byline(str(date), tourType(str(typ)), fmt.Sprintf("%.2f", num(weight))))),
	}
	if merged {
		items := []ui.Item{ui.Text(S.Tournament.Venues(len(venues)) + " ")}
		for i, v := range venues {
			if i > 0 {
				items = append(items, ui.Text(", "))
			}
			if v.id == id {
				items = append(items, ui.Strong(ui.Text(orUntitled(v.name))))
			} else {
				items = append(items, link(fmt.Sprintf("/tournaments/%d/", v.id), orUntitled(v.name)))
			}
		}
		headKids = append(headKids, ui.Byline(ui.Inline(items...)))
	}
	teams := count(nTeams)
	if merged {
		var sum *int64
		if s.db.QueryRow("select sum(n_teams) from x.tournament where id in ("+ids+")").Scan(&sum) == nil && sum != nil {
			teams = *sum
		}
	}
	stats := []ui.Item{stat(S.Tournament.Stat.Teams(), itoa(teams))}
	if merged {
		stats = append(stats, stat(S.Tournament.Stat.Venues(), itoa(int64(len(venues)))))
	}
	questions := "—"
	if count(nQ) > 0 {
		questions = itoa(*nQ)
	}
	stats = append(stats, stat(S.Tournament.Stat.Questions(), questions))
	if truedl != nil {
		stats = append(stats, stat(S.Common.Col.Difficulty(), fmt.Sprintf("%.1f", *truedl)))
	}
	if llCf != nil || llIn != nil {
		stats = append(stats, stat(S.Tournament.Stat.LlCross(), opt(llCf, 3)), stat(S.Tournament.Stat.LlIn(), opt(llIn, 3)))
	}
	stats = append(stats, stat(S.Tournament.Stat.RhoForecast(), opt(spF, 3)), stat(S.Tournament.Stat.RhoOld(), opt(spO, 3)))
	rho := S.Tournament.Rho() + " "
	if count(nObs) > 0 {
		rho += S.Tournament.LlObs(itoa(*nObs))
	} else {
		rho += S.Tournament.LlNone()
	}
	out := []ui.Item{ui.Pagehead(headKids...), ui.Stats(stats...), ui.Hint(ui.Text(rho))}

	if q := s.questionStrip(id); q != nil {
		out = append(out, q)
	}
	field, err := s.field(id, ids, merged, venues, venueName)
	if err != nil {
		return "", nil, err
	}
	return title, append(out, field), nil
}

// questionStrip is the difficulty of each question as a bar.
func (s *site) questionStrip(id int64) ui.Node {
	rows, err := s.db.Query("select q, b from question_params where tournament_id = ? order by q", id)
	if err != nil {
		return nil
	}
	defer rows.Close()
	type q struct {
		n int64
		b float64
	}
	var qs []q
	lo, hi := math.MaxFloat64, -math.MaxFloat64
	for rows.Next() {
		var x q
		var b *float64
		if rows.Scan(&x.n, &b) != nil {
			continue
		}
		x.b = num(b)
		lo, hi = math.Min(lo, x.b), math.Max(hi, x.b)
		qs = append(qs, x)
	}
	// A pack whose difficulties never moved off the prior is every bar the same
	// height: a strip that says nothing. Say so instead of drawing it.
	if len(qs) == 0 || hi-lo <= 1e-3 {
		return nil
	}
	bars := make([]ui.Item, len(qs))
	for i, x := range qs {
		h := 12 + 88*(x.b-lo)/(hi-lo)
		bars[i] = ui.Qbar(ui.H(fmt.Sprintf("%.0f", h)), ui.Title(S.Tournament.Questions.Bar(itoa(x.n), fmt.Sprintf("%.2f", x.b))))
	}
	return ui.Section(
		ui.Subhead(ui.Text(S.Tournament.Questions.Title())),
		ui.Qbars(bars...),
		ui.Hint(ui.Text(S.Tournament.Questions.Hint(fmt.Sprintf("%.2f", lo), fmt.Sprintf("%.2f", hi)))),
	)
}

// field is the result table: every team, its forecast, what it took, what the
// lineup was worth and what the result did to the rating.
func (s *site) field(id int64, ids string, merged bool, venues []venue, venueName func(int64) string) (ui.Node, error) {
	have := columns(s.db, "team_events")
	haveValues := tableExists(s.db, "achieve_values")
	values, join := "null, null", ""
	if haveValues {
		values = "a.peak, a.gain"
		join = "left join achieve_values a on a.tournament_id = v.tournament_id and a.team_id = v.team_id"
	}
	rows, err := s.db.Query(fmt.Sprintf(`select v.team_id, m.name, v.forecast_place, v.actual_place, v.mu_before, v.mu_after,
		v.rated, r.score, r.name, v.s_r, %s, %s, %s, v.tournament_id, v.forecast
		from team_events v left join x.team m on m.id = v.team_id
		left join x.result_ r on r.tournament_id = v.tournament_id and r.team_id = v.team_id %s
		where v.tournament_id in (%s) order by v.actual_place`, col(have, "d_place"), col(have, "d_pull"), values, join, ids))
	if err != nil {
		return nil, err
	}
	var evs []fieldRow
	for rows.Next() {
		var r fieldRow
		var name, playedAs *string
		var rated *int64
		if err := rows.Scan(&r.team, &name, &r.forecast, &r.actual, &r.before, &r.after, &rated, &r.score, &playedAs,
			&r.lineup, &r.dPlace, &r.dPull, &r.value, &r.gain, &r.venue, &r.strength); err != nil {
			rows.Close()
			return nil, err
		}
		r.name, r.playedAs = str(name), str(playedAs)
		r.rated = rated == nil || *rated != 0
		evs = append(evs, r)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if merged {
		// The merged place is by questions taken across every venue, ties
		// sharing the mean place; the forecast is by strength, which the model
		// puts on one scale whichever city a team played in.
		scores := make([]*float64, len(evs))
		strengths := make([]*float64, len(evs))
		for i, v := range evs {
			if v.score != nil {
				x := float64(*v.score)
				scores[i] = &x
			}
			strengths[i] = v.strength
		}
		places, fplaces := rankDesc(scores), rankDesc(strengths)
		for i := range evs {
			evs[i].allActual, evs[i].allForecast = places[i], fplaces[i]
		}
		sort.SliceStable(evs, func(i, j int) bool {
			a, b := math.Inf(1), math.Inf(1)
			if evs[i].allActual != nil {
				a = *evs[i].allActual
			}
			if evs[j].allActual != nil {
				b = *evs[j].allActual
			}
			if a != b {
				return a < b
			}
			return evs[i].venue < evs[j].venue
		})
	} else {
		for i := range evs {
			evs[i].allActual, evs[i].allForecast = evs[i].actual, evs[i].forecast
		}
	}
	rosters := s.lineups(ids)

	section := []ui.Item{ui.Subhead(ui.Text(S.Tournament.Field.Title()))}
	unrated := 0
	for _, v := range evs {
		if !v.rated {
			unrated++
		}
	}
	if unrated > 0 {
		section = append(section, ui.Hint(ui.Text(S.Tournament.Field.Unrated(itoa(int64(unrated)), itoa(int64(len(evs)))))))
	}
	if len(evs) == 0 {
		return ui.Section(append(section, ui.Empty(ui.Text(S.Tournament.Field.Empty())))...), nil
	}
	// The three parts of the step, in display points. The floor is whatever the
	// placement step and the pull leave unexplained, and only a winner has one.
	parts := func(v fieldRow) (dp, dl, floor float64, ok bool) {
		if v.before == nil || v.after == nil || v.dPlace == nil || v.dPull == nil {
			return 0, 0, 0, false
		}
		total := s.d.bTeam * (*v.after - *v.before)
		dp, dl = s.d.bTeam**v.dPlace, s.d.bTeam**v.dPull
		return dp, dl, total - dp - dl, true
	}
	// Shown only when a pull moved some rating; otherwise Δ is the placement step.
	split, floor := false, false
	for _, v := range evs {
		if _, dl, fl, ok := parts(v); ok {
			split = split || math.Round(dl) != 0
			floor = floor || math.Round(fl) != 0
		}
	}
	if len(rosters) > 0 {
		section = append(section, ui.Toggle(ui.ToggleRoster, ui.Text(S.Tournament.Field.RosterToggle())))
	}
	if merged {
		opts := []ui.Item{ui.Label(S.Tournament.Field.Venue()), ui.Option(ui.Value(""), ui.Text(S.Tournament.Field.AllVenues()))}
		for _, v := range venues {
			k := 0
			for _, x := range evs {
				if x.venue == v.id {
					k++
				}
			}
			opts = append(opts, ui.Option(ui.Value(itoa(v.id)), ui.Text(S.Tournament.Field.VenueOption(orUntitled(v.name), itoa(int64(k))))))
		}
		section = append(section, ui.Venuefilter(opts...))
	}

	headRow := []ui.Item{
		hcell(S.Common.Col.Place(), ui.Num()),
		hcell(S.Tournament.Field.Forecast(), ui.Num()),
		hcell(S.Common.Col.Team(), ui.PinCol()),
	}
	if merged {
		headRow = append(headRow, hcell(S.Tournament.Field.VenueCol()))
	}
	headRow = append(headRow,
		hcell(S.Common.Col.Taken(), ui.Num()),
		hcell(S.Tournament.Field.Lineup(), ui.Num(), ui.Title(S.Tournament.Field.LineupTitle())),
		hcell(S.Common.Col.Value(), ui.Num(), ui.Title(S.Tournament.Field.ValueTitle())),
	)
	if haveValues {
		headRow = append(headRow, hcell(S.Common.Col.Gain(), ui.Num(), ui.Title(S.Tournament.Field.GainTitle())))
	} else {
		headRow = append(headRow, hcell(S.Common.Col.Before(), ui.Num()), hcell(S.Common.Col.After(), ui.Num()), hcell("Δ", ui.Num()))
	}
	if split {
		headRow = append(headRow, hcell(S.Tournament.Field.ByPlace(), ui.Num(), ui.Title(S.Tournament.Field.ByPlaceTitle())), hcell(S.Tournament.Field.ToLineup(), ui.Num(), ui.Title(S.Tournament.Field.ToLineupTitle())))
	}
	if floor {
		headRow = append(headRow, hcell(S.Tournament.Field.Winner(), ui.Num(), ui.Title(S.Tournament.Field.WinnerTitle())))
	}
	table := []ui.Item{ui.Scroll(), ui.Trow(headRow...)}

	beat := func(fc, a *float64) []ui.Item {
		switch {
		case fc == nil || a == nil:
		case *a < *fc-0.5:
			return []ui.Item{ui.PlaceBetter}
		case *a > *fc+0.5:
			return []ui.Item{ui.PlaceWorse}
		}
		return nil
	}
	for _, v := range evs {
		row := []ui.Item{ui.Data("venue", itoa(v.venue))}
		// A merged field shows the places over every venue; the filter swaps in
		// the venue's own, which each cell carries beside them.
		if merged {
			own := "num"
			if b := beat(v.forecast, v.actual); b != nil {
				own += " " + map[ui.Attr]string{ui.PlaceBetter: "place-better", ui.PlaceWorse: "place-worse"}[b[0].(ui.Attr)]
			}
			row = append(row,
				numCell(place(v.allActual), append(beat(v.allForecast, v.allActual), ui.Data("own", place(v.actual)), ui.Data("own-class", own))...),
				numCell(place(v.allForecast), ui.Faint(), ui.Data("own", place(v.forecast))),
			)
		} else {
			row = append(row, numCell(place(v.actual), beat(v.forecast, v.actual)...), numCell(place(v.forecast), ui.Faint()))
		}
		// The link says the team's own name; the name it entered under, when it
		// is a different one, goes next to it.
		shown := v.name
		if shown == "" {
			shown = v.playedAs
		}
		teamLine := []ui.Item{link(fmt.Sprintf("/teams/%d/", v.team), orDash(shown))}
		if v.playedAs != "" && fold(v.playedAs) != fold(shown) {
			teamLine = append(teamLine, ui.Text(" "), ui.Muted(ui.Text(S.Tournament.Field.PlayedAs(v.playedAs))))
		}
		teamCell := []ui.Item{ui.PinCol(), line(teamLine...)}
		if sub := subline(unratedFlag(v.rated)); sub != nil {
			teamCell = append(teamCell, sub)
		}
		// The lineup sits inside the team's own cell, one player to a line, and
		// opens under the name: by the switch beside it or the one above the table.
		if roster := rosters[[2]int64{v.venue, v.team}]; len(roster) > 0 {
			members := make([]ui.Item, len(roster))
			for i, m := range roster {
				var badge ui.Item
				if m.provisional {
					badge = provBadge()
				}
				items := []ui.Item{link(fmt.Sprintf("/players/%d/", m.player), orDash(m.name)), space(badge), badge, ui.Text(" "), ui.Fig(ui.Text(dispOpt(m.before)))}
				if m.before != nil && m.after != nil && math.Round(*m.after-*m.before) != 0 {
					d := *m.after - *m.before
					tone := ui.AmountPositive
					if d < 0 {
						tone = ui.AmountNegative
					}
					items = append(items, ui.Text(" "), ui.Amount(tone, ui.Text(signed(d))))
				}
				members[i] = ui.Member(line(items...))
			}
			teamCell = append(teamCell, ui.Rosteropen(), ui.Cellroster(members...))
		}
		row = append(row, ui.Cell(teamCell...))
		if merged {
			row = append(row, ui.Cell(line(link(fmt.Sprintf("/tournaments/%d/", v.venue), orUntitled(venueName(v.venue)))), subline(ui.Text(S.Tournament.Field.VenuePlace(place(v.actual))))))
		}
		taken := "—"
		if v.score != nil {
			taken = itoa(*v.score)
		}
		lineup := "—"
		if v.lineup != nil {
			lineup = disp(s.d.aTeam + s.d.bTeam**v.lineup)
		}
		row = append(row, numCell(taken), numCell(lineup, ui.Faint()), numCell(dispOpt(v.value)))
		if haveValues {
			row = append(row, gainCell(v.gain))
		} else {
			row = append(row, stepCells(s.d.aTeam, s.d.bTeam, v.before, v.after)...)
		}
		dp, dl, fl, ok := parts(v)
		if split {
			if ok {
				row = append(row, numCell(signed(dp), toneOf(dp)), numCell(signed(dl), toneOf(dl)))
			} else {
				row = append(row, numCell("—", ui.Faint()), numCell("—", ui.Faint()))
			}
		}
		if floor {
			if ok && math.Round(fl) != 0 {
				row = append(row, numCell(signed(fl), toneOf(fl)))
			} else {
				row = append(row, emptyCell(ui.Num(), ui.Faint()))
			}
		}
		table = append(table, ui.Trow(row...))
	}
	section = append(section, ui.Table(table...))

	notes := []string{S.Tournament.Field.NoteLineup()}
	if haveValues {
		notes = append(notes, S.Tournament.Field.NoteValue())
	}
	if split {
		notes = append(notes, S.Tournament.Field.NoteSplit())
	}
	if floor {
		notes = append(notes, S.Tournament.Field.NoteFloor())
	}
	if len(rosters) > 0 {
		notes = append(notes, S.Tournament.Field.NoteRoster())
	}
	if merged {
		notes = append(notes, S.Tournament.Field.NoteMerged())
	}
	section = append(section, ui.Hint(ui.Text(strings.Join(notes, " "))))
	return ui.Section(section...), nil
}

// rankDesc places each value in its field, the largest first, ties sharing the
// mean of the places they span. A missing value has no place.
func rankDesc(xs []*float64) []*float64 {
	out := make([]*float64, len(xs))
	for i, x := range xs {
		if x == nil {
			continue
		}
		above, level := 0.0, 0.0
		for _, y := range xs {
			if y == nil {
				continue
			}
			if *y > *x {
				above++
			} else if *y == *x {
				level++
			}
		}
		p := above + (level+1)/2
		out[i] = &p
	}
	return out
}

// lineups are the players at each table, keyed by (venue, team), strongest
// first, with each player's published number on the morning of the game.
func (s *site) lineups(ids string) map[[2]int64][]member {
	out := map[[2]int64][]member{}
	rows, err := s.db.Query(`select g.team_id, g.player_id, p.name, v.theta_before, v.theta_after, v.phi_before,
		v.sigma_before, g.tournament_id from x.player_game g left join x.player p on p.id = g.player_id
		left join player_events v on v.player_id = g.player_id and v.tournament_id = g.tournament_id
		where g.tournament_id in (` + ids + `)`)
	if err != nil {
		return out
	}
	defer rows.Close()
	for rows.Next() {
		var team, player, tid int64
		var name *string
		var theta, after, phi, sigma *float64
		if rows.Scan(&team, &player, &name, &theta, &after, &phi, &sigma, &tid) != nil {
			continue
		}
		// The published player number is the strength at an in-person table,
		// theta + phi, and that is what the roster lists.
		m := member{player: player, name: str(name), provisional: sigma != nil && *sigma > s.d.provSigma}
		if theta != nil {
			x := s.d.aPlayer + s.d.bPlayer*(*theta+num(phi))
			m.before = &x
		}
		if after != nil {
			x := s.d.aPlayer + s.d.bPlayer*(*after+num(phi))
			m.after = &x
		}
		k := [2]int64{tid, team}
		out[k] = append(out[k], m)
	}
	for _, v := range out {
		sort.SliceStable(v, func(i, j int) bool {
			a, b := math.Inf(-1), math.Inf(-1)
			if v[i].before != nil {
				a = *v[i].before
			}
			if v[j].before != nil {
				b = *v[j].before
			}
			if a != b {
				return a > b
			}
			return v[i].name < v[j].name
		})
	}
	return out
}
