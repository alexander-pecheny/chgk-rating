package main

import (
	"fmt"
	"math"

	"ratingweb/ui"
)

// The digits a tournament's metrics print to.
const metricDigits = 3

func (s *site) tournamentPage(id int64) (string, []ui.Item, error) {
	t, ok, err := s.q.tournament(id)
	if !ok || err != nil {
		return "", nil, orNotFound(err)
	}
	// One package played in several cities is one field: whichever venue the
	// reader opens, the page shows every venue's teams, ranked together.
	venues, err := s.q.venues(id)
	if err != nil {
		return "", nil, err
	}
	ids := []int64{id}
	if len(venues) > 1 {
		ids = venueIDs(venues)
	}
	out, err := s.tournamentHead(id, t, venues, ids)
	if err != nil {
		return "", nil, err
	}
	body, err := s.tournamentBody(id, ids, venues)
	return tournamentTitle(t), append(out, body...), err
}

func tournamentTitle(t tourHead) string {
	if str(t.name) == "" {
		return S.Tournament.Untitled()
	}
	return *t.name
}

func venueIDs(venues []venue) []int64 {
	ids := make([]int64, len(venues))
	for i, v := range venues {
		ids[i] = v.id
	}
	return ids
}

// tournamentHead is the byline, the venues of a merged field, the stats and
// the note on the forecast's quality.
func (s *site) tournamentHead(id int64, t tourHead, venues []venue, ids []int64) ([]ui.Item, error) {
	weight := fmt.Sprintf("%.2f", num(t.weight))
	kids := []ui.Item{ui.Byline(ui.Text(S.Tournament.Byline(str(t.date), tourType(str(t.typ)), weight)))}
	teams := count(t.nTeams)
	if len(venues) > 1 {
		kids = append(kids, venueLine(id, venues))
		sum, err := s.q.fieldTeams(ids)
		if err != nil {
			return nil, err
		}
		if sum != nil {
			teams = *sum
		}
	}
	return []ui.Item{ui.Pagehead(kids...), ui.Stats(tournamentStats(t, teams, venues)...)}, nil
}

// venueLine names every venue of a merged field, this one in bold.
func venueLine(id int64, venues []venue) ui.Node {
	items := []ui.Item{ui.Text(S.Tournament.Venues(len(venues)) + " ")}
	for i, v := range venues {
		if i > 0 {
			items = append(items, ui.Text(", "))
		}
		if v.id == id {
			items = append(items, ui.Strong(ui.Text(orUntitled(v.name))))
		} else {
			items = append(items, link(tourHref(v.id), orUntitled(v.name)))
		}
	}
	return ui.Byline(ui.Inline(items...))
}

func tournamentStats(t tourHead, teams int64, venues []venue) []ui.Item {
	stats := []ui.Item{stat(S.Tournament.Stat.Teams(), itoa(teams))}
	if len(venues) > 1 {
		stats = append(stats, stat(S.Tournament.Stat.Venues(), itoa(int64(len(venues)))))
	}
	questions := dash
	if count(t.nQ) > 0 {
		questions = itoa(*t.nQ)
	}
	stats = append(stats, stat(S.Tournament.Stat.Questions(), questions))
	if t.truedl != nil {
		stats = append(stats, stat(S.Common.Col.Difficulty(), fmt.Sprintf("%.1f", *t.truedl)))
	}
	return append(stats, forecastStats(t)...)
}

// forecastStats are how well the model saw the tournament coming.
func forecastStats(t tourHead) []ui.Item {
	var stats []ui.Item
	if t.llCf != nil || t.llIn != nil {
		stats = append(stats, stat(S.Tournament.Stat.LlCross(), opt(t.llCf, metricDigits)),
			stat(S.Tournament.Stat.LlIn(), opt(t.llIn, metricDigits)))
	}
	return append(stats, stat(S.Tournament.Stat.RhoForecast(), opt(t.spF, metricDigits)),
		stat(S.Tournament.Stat.RhoOld(), opt(t.spO, metricDigits)))
}

// tournamentBody is the question strip, when there is one, and the field.
func (s *site) tournamentBody(id int64, ids []int64, venues []venue) ([]ui.Item, error) {
	qs, err := s.q.questions(id)
	if err != nil {
		return nil, err
	}
	var out []ui.Item
	if q := questionStrip(qs); q != nil {
		out = append(out, q)
	}
	fc, err := s.q.forecastQuality(id)
	if err != nil {
		return nil, err
	}
	if q := forecastSection(fc); q != nil {
		out = append(out, q)
	}
	f, err := s.field(ids, venues)
	return append(out, f), err
}

// forecastSection is every system's forecast quality on this tournament, in
// the clean test window only; none outside it.
func forecastSection(rows []forecastRow) ui.Node {
	if len(rows) == 0 {
		return nil
	}
	names := map[string]string{"ours": S.Tournament.Forecast.Ours(), "fun": S.Tournament.Forecast.Fun(), "gg": S.Tournament.Forecast.Gg(), "quest": S.Tournament.Forecast.Quest()}
	var body []ui.Item
	for _, r := range rows {
		name := names[r.system]
		if name == "" {
			continue
		}
		body = append(body, ui.Trow(textCell(name), numCell(fmt.Sprintf("%.3f", r.ndcg)), numCell(fmt.Sprintf("%.3f", r.ndcg3)), numCell(itoa(r.n))))
	}
	if len(body) == 0 {
		return nil
	}
	header := []ui.Item{nameCol(S.Tournament.Forecast.ColSystem()), numCol("NDCG"), numCol("NDCG@3"), numCol(S.Tournament.Forecast.ColTeams())}
	return section(S.Tournament.Forecast.Title(), table(header, body...))
}

// The question strip's bars run from barMin to barMin+barSpan percent tall.
const (
	barMin  = 12
	barSpan = 88
)

// questionStrip is the difficulty of each question as a bar. A pack whose
// difficulties never moved off the prior is every bar the same height: a
// strip that says nothing, so there is none.
func questionStrip(qs []question) ui.Node {
	lo, hi := math.MaxFloat64, -math.MaxFloat64
	for _, x := range qs {
		lo, hi = math.Min(lo, num(x.b)), math.Max(hi, num(x.b))
	}
	if len(qs) == 0 || hi-lo <= 1e-3 {
		return nil
	}
	bars := make([]ui.Item, len(qs))
	for i, x := range qs {
		h := barMin + barSpan*(num(x.b)-lo)/(hi-lo)
		bars[i] = ui.Qbar(ui.H(fmt.Sprintf("%.0f", h)), ui.Title(S.Tournament.Questions.Bar(itoa(x.n), fmt.Sprintf("%.2f", num(x.b)))))
	}
	hint := S.Tournament.Questions.Hint(fmt.Sprintf("%.2f", lo), fmt.Sprintf("%.2f", hi))
	return section(S.Tournament.Questions.Title(), ui.Qbars(bars...), ui.Hint(ui.Text(hint)))
}
