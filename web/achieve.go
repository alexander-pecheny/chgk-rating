package main

import (
	"fmt"
	"math"
	"strconv"

	"ratingweb/ui"
)

// The achievement rating on a team's page: the results it is the sum of, and
// every result with what it was worth.

// ruleOr is a rule as published, or def when the rules do not say.
func ruleOr(rules map[string]string, key, def string) string {
	if v, ok := rules[key]; ok {
		return v
	}
	return def
}

// breakdown is how a team's rating is summed: over how many slots.
type breakdown struct {
	achieved
	slots string
}

// achieveBreakdown is the rating taken apart: the results it is the sum of,
// each with what it was worth on the day, how much of that is left, and what
// it adds.
func (s *site) achieveBreakdown(id int64, a achieved, rules map[string]string, at *release) (ui.Node, error) {
	parts, err := s.q.parts(id, at)
	if err != nil {
		return nil, err
	}
	b := breakdown{a, ruleOr(rules, "top_x", fmt.Sprintf("%.0f", a.divisor))}
	rows := make([]ui.Item, 0, len(parts))
	sum := 0.0
	for _, p := range parts {
		sum += p.contrib
		rows = append(rows, b.partRow(p))
	}
	rows = append(append(rows, b.emptySlots(len(parts))...), b.footRow(sum))
	if row := strengthRow(a.rating, rules); row != nil {
		rows = append(rows, row)
	}
	t := ui.Table(append([]ui.Item{ui.Scroll(), ui.Breakdown(), ui.Trow(breakdownCols()...)}, rows...)...)
	return section(S.Team.Breakdown.Title(), t), nil
}

func breakdownCols() []ui.Item {
	return []ui.Item{
		nameCol(S.Common.Col.Tournament()),
		numCol(S.Common.Col.Place()),
		numCol(S.Common.Col.Value(), S.Common.Col.ValueTitle()),
		numCol(S.Team.Breakdown.Left(), S.Team.Breakdown.LeftTitle()),
		numCol(S.Team.Breakdown.Now(), S.Team.Breakdown.NowTitle()),
	}
}

func (b breakdown) partRow(p part) ui.Node {
	// One run of text: the line under a name is a flex row, and two text
	// pieces side by side in it would lose the space between them.
	under := p.date
	if p.nv > 1 {
		under += " · " + S.Team.Breakdown.Merged(placeOf(p.placeAll), itoa(p.n), placeOf(p.placeOwn), itoa(p.nOwn))
	}
	name := ui.Cell(ui.PinCol(), line(link(tourHref(p.tid), orUntitled(p.name))), subline(ui.Text(under)), ui.How(ui.Inline(b.how(p)...)))
	return ui.Trow(
		name,
		placeOfN(placeOf(p.placeAll), p.n),
		numCell(disp(p.peak)),
		numCell(pct(p.fade*p.ov)+"%", ui.Faint(), ui.Title(leftTitle(p))),
		numCell(disp(p.contrib*b.divisor), ui.Bold()),
	)
}

// leftTitle says what time took off a result, and what the overlap with
// another result of the same week did.
func leftTitle(p part) string {
	if p.ov < 0.995 {
		return S.Team.Breakdown.FadeOverlap(pct(p.fade), pct(p.ov))
	}
	return S.Team.Breakdown.Fade(pct(p.fade))
}

// how is how the rating points on the day were made, in one line under the
// name, in the letters of the formula on /method/: every factor, one or not.
// The points themselves are the next column's.
func (b breakdown) how(p part) []ui.Item {
	terms := [][]ui.Item{
		{ui.Text("S"), ui.Sup(ui.Text("+")), ui.Text(S.Team.Breakdown.Equals(disp(max(0, p.strength))))},
		{ui.Text(S.Team.Breakdown.Place(placeOf(p.placeAll)))},
	}
	for _, w := range []struct {
		name string
		v    float64
	}{{S.Team.Breakdown.WFormat(), p.wfmt}, {S.Team.Breakdown.WTop(), p.wfield}, {S.Team.Breakdown.WTowns(), p.wreach}, {S.Team.Breakdown.WLegion(), p.wlegion}} {
		terms = append(terms, []ui.Item{ui.Text(S.Team.Breakdown.W()), ui.Sub(ui.Text(w.name)), ui.Text(S.Team.Breakdown.Equals(w2(w.v)))})
	}
	var how []ui.Item
	for i, t := range terms {
		if i < len(terms)-1 {
			t = append(t, ui.Text(","))
		}
		// The space between terms is where a narrow line may break.
		how = append(how, ui.Term(ui.Inline(t...)), ui.Text(" "))
	}
	return how
}

// emptySlots are the slots no result fills yet, each worth nothing.
func (b breakdown) emptySlots(parts int) []ui.Item {
	slots, err := strconv.Atoi(b.slots)
	if err != nil {
		slots = parts
	}
	var out []ui.Item
	for i := parts; i < slots; i++ {
		out = append(out, ui.Trow(
			textCell(S.Team.Breakdown.EmptySlot(), ui.Faint()), emptyCell(), emptyCell(), emptyCell(),
			numCell("0", ui.Faint()),
		))
	}
	return out
}

func (b breakdown) footRow(sum float64) ui.Node {
	text := S.Team.Breakdown.Sum(disp(sum*b.divisor), fmt.Sprintf("%.0f", b.divisor))
	return ui.Trow(ui.FootRow(), textCell(text, ui.Colspan("4")), numCell(disp(b.rating)))
}

// strengthRow is the rating under the points: the strength they stand for,
// strength_a·points^strength_p, with the constants the achievement rule fixed.
func strengthRow(points float64, rules map[string]string) ui.Node {
	a, err1 := strconv.ParseFloat(ruleOr(rules, "strength_a", "0"), 64)
	p, err2 := strconv.ParseFloat(ruleOr(rules, "strength_p", "0"), 64)
	if err1 != nil || err2 != nil || p <= 0 || points <= 0 {
		return nil
	}
	text := S.Team.Breakdown.Strength(disp(a), strconv.FormatFloat(p, 'f', -1, 64))
	return ui.Trow(ui.FootRow(), textCell(text, ui.Colspan("4")), numCell(disp(a*math.Pow(points, p)), ui.Bold()))
}

// teamResults is every result the team has played, with what it was worth and
// whether it counts today.
func (s *site) teamResults(id int64, at *release) (ui.Node, error) {
	evs, err := s.q.results(id)
	if err != nil {
		return nil, err
	}
	evs = before(evs, at, func(e result) string { return str(e.date) })
	day := s.releaseDate
	if at != nil {
		day = at.date
	}
	title := S.Team.Results.Title(itoa(int64(len(evs))))
	if len(evs) == 0 {
		return section(title, ui.Empty(ui.Text(S.Common.NoTournaments()))), nil
	}
	cutoff := cutoff(day)
	rows := make([]ui.Item, len(evs))
	for i, e := range evs {
		rows[i] = resultRow(e, cutoff)
	}
	kids := olderToggle(evs, cutoff)
	return section(title, append(kids, table(resultCols(), rows...))...), nil
}

// cutoff is the date three years before the release day: older results are
// folded away behind the switch.
func cutoff(day string) string {
	const (
		yearLen = len("2006")
		years   = 3
	)
	y, err := strconv.Atoi(prefix(day, yearLen))
	if err != nil || len(day) < yearLen {
		return ""
	}
	return strconv.Itoa(y-years) + day[yearLen:]
}

// olderToggle is the switch for the results before the cutoff, when there are any.
func olderToggle(evs []result, cutoff string) []ui.Item {
	older := 0
	for _, e := range evs {
		if str(e.date) < cutoff {
			older++
		}
	}
	if older == 0 {
		return nil
	}
	count := ui.Count(ui.Text(S.Team.Results.OlderCount(itoa(int64(older)), cutoff)))
	return []ui.Item{ui.Toggle(ui.ToggleOlder, ui.Inline(ui.Text(S.Team.Results.OlderToggle()+" "), count))}
}

func resultCols() []ui.Item {
	return append(tourCols(),
		numCol(S.Common.Col.Place()),
		numCol(S.Common.Col.Value(), S.Common.Col.ValueTitle()),
		numCol(S.Team.Results.Counts(), S.Team.Results.CountsColTitle()),
		numCol(S.Common.Col.Gain(), S.Common.Col.GainTitle()),
	)
}

func resultRow(e result, cutoff string) ui.Node {
	row := append(tourCells(str(e.date), e.tid, str(e.name), str(e.typ), counted(e.rated)),
		resultPlace(e), resultValue(e), countsCell(e.counts), gainCell(e.gain))
	switch {
	case e.counts:
		row = append(row, ui.CountsRow())
	case str(e.date) < cutoff:
		row = append(row, ui.OlderRow())
	}
	return ui.Trow(row...)
}

func resultPlace(e result) ui.Node {
	if e.nTeams == nil {
		return ui.Cell(ui.Num(), ui.NoWrap(), ui.Text(place(e.actual)))
	}
	return placeOfN(place(e.actual), *e.nTeams)
}

// resultValue is what the result was worth, with how on hover.
func resultValue(e result) ui.Node {
	value := []ui.Item{ui.Num(), ui.Text(dispOpt(e.peak))}
	if e.strength == nil || e.w == nil || e.placeVal == nil {
		return ui.Cell(value...)
	}
	t := S.Team.Results.ValueHow(disp(max(0, *e.strength)), placeOf(*e.placeVal), w2(*e.w))
	return ui.Cell(append(value, ui.Title(t))...)
}

func countsCell(counts bool) ui.Node {
	if counts {
		return ui.Cell(ui.Num(), ui.Inline(ui.Dotin()))
	}
	return emptyCell(ui.Num())
}
