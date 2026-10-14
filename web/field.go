package main

import (
	"math"
	"sort"

	"ratingweb/ui"
)

// fieldView is a tournament's result table: every team, its forecast, what it
// took, what the lineup was worth and what the result did to the rating.
type fieldView struct {
	evs     []fieldRow
	venues  []venue
	rosters map[[2]int64][]member
	// merged is a field of several venues; values is the achievement value
	// of each result, in place of the step of the rating.
	merged, values bool
	// split shows the step as placement and pull, when a pull moved some
	// rating; floor shows what a winner got beyond both.
	split, floor bool
}

func (s *site) field(ids []int64, venues []venue) (ui.Node, error) {
	evs, err := s.q.field(ids)
	if err != nil {
		return nil, err
	}
	rosters, err := s.q.lineups(ids)
	if err != nil {
		return nil, err
	}
	f := fieldView{evs: evs, venues: venues, rosters: rosters, merged: len(venues) > 1, values: s.q.achieve}
	f.placeAll()
	f.split, f.floor = stepParts(evs)
	return f.section(), nil
}

// placeAll puts every team in the order of the places over the whole field.
// The merged place is by questions taken across every venue, ties sharing the
// mean place; the forecast is by strength, which the model puts on one scale
// whichever city a team played in.
func (f *fieldView) placeAll() {
	evs := f.evs
	if !f.merged {
		for i := range evs {
			evs[i].allActual, evs[i].allForecast = evs[i].actual, evs[i].forecast
		}
		return
	}
	scores, strengths := make([]*float64, len(evs)), make([]*float64, len(evs))
	for i, v := range evs {
		// The merged field counts only the questions every venue played.
		if v.fieldScore != nil {
			evs[i].score = v.fieldScore
		}
		scores[i], strengths[i] = scoreOf(evs[i].score), v.strength
	}
	places, fplaces := rankDesc(scores), rankDesc(strengths)
	for i := range evs {
		evs[i].allActual, evs[i].allForecast = places[i], fplaces[i]
	}
	sort.SliceStable(evs, func(i, j int) bool { return placedBefore(evs[i], evs[j]) })
}

func scoreOf(score *int64) *float64 {
	if score == nil {
		return nil
	}
	x := float64(*score)
	return &x
}

// placedBefore orders a merged field: by place, a team with none last, then
// by venue.
func placedBefore(x, y fieldRow) bool {
	a, b := orHighest(x.allActual), orHighest(y.allActual)
	if a != b {
		return a < b
	}
	return x.venue < y.venue
}

// orHighest is a nullable number, or infinity for none.
func orHighest(p *float64) float64 {
	if p == nil {
		return math.Inf(1)
	}
	return *p
}

// parts are the three parts of the step, in display points. The floor is
// whatever the placement step and the pull leave unexplained, and only a
// winner has one.
func parts(v fieldRow) (dp, dl, floor float64, ok bool) {
	if v.step == nil || v.dPlace == nil || v.dPull == nil {
		return 0, 0, 0, false
	}
	dp, dl = *v.dPlace, *v.dPull
	return dp, dl, *v.step - dp - dl, true
}

// stepParts says which parts of the step any team shows.
func stepParts(evs []fieldRow) (split, floor bool) {
	for _, v := range evs {
		if _, dl, fl, ok := parts(v); ok {
			split = split || math.Round(dl) != 0
			floor = floor || math.Round(fl) != 0
		}
	}
	return split, floor
}

func (f fieldView) section() ui.Node {
	kids := []ui.Item{}
	if len(f.evs) == 0 {
		return section(S.Tournament.Field.Title(), append(kids, ui.Empty(ui.Text(S.Tournament.Field.Empty())))...)
	}
	kids = append(kids, f.switches()...)
	rows := make([]ui.Item, len(f.evs))
	for i, v := range f.evs {
		rows[i] = f.row(v)
	}
	kids = append(kids, table(f.header(), rows...))
	return section(S.Tournament.Field.Title(), kids...)
}

// switches open the lineups and pick a venue of a merged field.
func (f fieldView) switches() []ui.Item {
	var out []ui.Item
	if len(f.rosters) > 0 {
		out = append(out, ui.Toggle(ui.ToggleRoster, ui.Text(S.Tournament.Field.RosterToggle())))
	}
	if !f.merged {
		return out
	}
	opts := []ui.Item{ui.Label(S.Tournament.Field.Venue()), ui.Option(ui.Value(""), ui.Text(S.Tournament.Field.AllVenues()))}
	for _, v := range f.venues {
		label := S.Tournament.Field.VenueOption(orUntitled(v.name), itoa(int64(f.teamsAt(v.id))))
		opts = append(opts, ui.Option(ui.Value(itoa(v.id)), ui.Text(label)))
	}
	return append(out, ui.Venuefilter(opts...))
}

func (f fieldView) teamsAt(venue int64) int {
	n := 0
	for _, x := range f.evs {
		if x.venue == venue {
			n++
		}
	}
	return n
}

func (f fieldView) header() []ui.Item {
	cols := []ui.Item{numCol(S.Common.Col.Place()), numCol(S.Tournament.Field.Forecast()), nameCol(S.Common.Col.Team())}
	if f.merged {
		cols = append(cols, hcell(S.Tournament.Field.VenueCol()))
	}
	cols = append(cols, numCol(S.Common.Col.Taken()), numCol(S.Tournament.Field.Lineup(), S.Tournament.Field.LineupTitle()), numCol(S.Common.Col.Value(), S.Tournament.Field.ValueTitle()))
	if f.values {
		cols = append(cols, numCol(S.Common.Col.Gain(), S.Tournament.Field.GainTitle()))
	} else {
		cols = append(cols, stepCols()...)
	}
	if f.split {
		cols = append(cols, numCol(S.Tournament.Field.ByPlace(), S.Tournament.Field.ByPlaceTitle()), numCol(S.Tournament.Field.ToLineup(), S.Tournament.Field.ToLineupTitle()))
	}
	if f.floor {
		cols = append(cols, numCol(S.Tournament.Field.Winner(), S.Tournament.Field.WinnerTitle()))
	}
	return cols
}

func (f fieldView) row(v fieldRow) ui.Node {
	row := append([]ui.Item{ui.Data("venue", itoa(v.venue))}, f.placeCells(v)...)
	row = append(row, ui.Cell(f.teamCell(v)...))
	if f.merged {
		where := link(tourHref(v.venue), orUntitled(f.venueName(v.venue)))
		row = append(row, ui.Cell(line(where), subline(ui.Text(S.Tournament.Field.VenuePlace(place(v.actual))))))
	}
	row = append(row, numCell(intOpt(v.score)), numCell(dispOpt(v.lineup), ui.Faint()), numCell(dispOpt(v.value)))
	if f.values {
		row = append(row, gainCell(v.gain))
	} else {
		row = append(row, stepCells(v.before, v.after)...)
	}
	return ui.Trow(append(row, f.partCells(v)...)...)
}

// beat is the tone of a place against its forecast, when it is off by more
// than half a place.
func beat(fc, a *float64) []ui.Item {
	const slack = 0.5
	switch {
	case fc == nil || a == nil:
	case *a < *fc-slack:
		return []ui.Item{ui.PlaceBetter}
	case *a > *fc+slack:
		return []ui.Item{ui.PlaceWorse}
	}
	return nil
}

// placeCells are the place and the forecast. A merged field shows the places
// over every venue; the filter swaps in the venue's own, which each cell
// carries beside them.
func (f fieldView) placeCells(v fieldRow) []ui.Item {
	if !f.merged {
		return []ui.Item{numCell(place(v.actual), beat(v.forecast, v.actual)...), numCell(place(v.forecast), ui.Faint())}
	}
	own := "num"
	if b := beat(v.forecast, v.actual); b != nil {
		own += " " + map[ui.Attr]string{ui.PlaceBetter: "place-better", ui.PlaceWorse: "place-worse"}[b[0].(ui.Attr)]
	}
	actual := append(beat(v.allForecast, v.allActual), ui.Data("own", place(v.actual)), ui.Data("own-class", own))
	return []ui.Item{numCell(place(v.allActual), actual...), numCell(place(v.allForecast), ui.Faint(), ui.Data("own", place(v.forecast)))}
}

// teamCell links the team's own name; the name it entered under, when it is
// a different one, goes next to it. The lineup sits inside the cell, one
// player to a line, and opens under the name: by the switch beside it or the
// one above the table.
func (f fieldView) teamCell(v fieldRow) []ui.Item {
	shown := v.name
	if shown == "" {
		shown = v.playedAs
	}
	name := []ui.Item{link(teamHref(v.team), orDash(shown))}
	if v.playedAs != "" && fold(v.playedAs) != fold(shown) {
		name = append(name, ui.Text(" "), ui.Muted(ui.Text(S.Tournament.Field.PlayedAs(v.playedAs))))
	}
	flag := unratedFlag(v.rated)
	cell := []ui.Item{ui.PinCol(), line(append(name, space(flag), flag)...)}
	if roster := f.rosters[[2]int64{v.venue, v.team}]; len(roster) > 0 {
		cell = append(cell, ui.Rosteropen(), ui.Cellroster(lineup(roster)...))
	}
	return cell
}

// lineup is the players at a table, each with the move of their number.
func lineup(roster []member) []ui.Item {
	out := make([]ui.Item, len(roster))
	for i, m := range roster {
		out[i] = memberLine(m.player, orDash(m.name), m.provisional, m.before, moved(m)...)
	}
	return out
}

func moved(m member) []ui.Item {
	if m.before == nil || m.after == nil || math.Round(*m.after-*m.before) == 0 {
		return nil
	}
	d := *m.after - *m.before
	tone := ui.AmountPositive
	if d < 0 {
		tone = ui.AmountNegative
	}
	return []ui.Item{ui.Text(" "), ui.Amount(tone, ui.Text(signed(d)))}
}

func (f fieldView) venueName(id int64) string {
	for _, x := range f.venues {
		if x.id == id {
			return x.name
		}
	}
	return ""
}

// partCells are the step taken apart, in the columns the field shows.
func (f fieldView) partCells(v fieldRow) []ui.Item {
	var out []ui.Item
	dp, dl, fl, ok := parts(v)
	switch {
	case f.split && ok:
		out = append(out, deltaCell(dp), deltaCell(dl))
	case f.split:
		out = append(out, dashCell(), dashCell())
	}
	switch {
	case f.floor && ok && math.Round(fl) != 0:
		out = append(out, deltaCell(fl))
	case f.floor:
		out = append(out, emptyCell(ui.Num(), ui.Faint()))
	}
	return out
}

// rankDesc places each value in its field, the largest first, ties sharing the
// mean of the places they span. A missing value has no place.
func rankDesc(xs []*float64) []*float64 {
	out := make([]*float64, len(xs))
	for i, x := range xs {
		if x != nil {
			p := placeAmong(*x, xs)
			out[i] = &p
		}
	}
	return out
}

func placeAmong(x float64, xs []*float64) float64 {
	above, level := 0.0, 0.0
	for _, y := range xs {
		switch {
		case y == nil:
		case *y > x:
			above++
		case *y == x:
			level++
		}
	}
	return above + (level+1)/2
}
