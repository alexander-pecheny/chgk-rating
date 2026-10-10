package main

import (
	"net/http"
	"strconv"

	"ratingweb/ui"
)

// A player's or a team's page as of a past release, at ?release=id: the
// numbers of that release, the history up to it and the results played
// before its date.

// releaseParam is the release a page asks for: nil for the current one,
// which is also what the last release's id means; ok is false for an id the
// index does not know.
func (s *site) releaseParam(r *http.Request) (at *release, ok bool) {
	v := r.URL.Query().Get("release")
	if v == "" {
		return nil, true
	}
	id, err := strconv.ParseInt(v, decimal, idBits)
	i, known := s.relIdx[id]
	if err != nil || !known {
		return nil, false
	}
	if i == len(s.rels)-1 {
		return nil, true
	}
	return &s.rels[i], true
}

// asOfHref is an entity page as of a release; nil is the current page.
func asOfHref(href string, at *release) string {
	if at == nil {
		return href
	}
	return href + "?release=" + itoa(at.id)
}

// before keeps the rows dated before the release: a release reads only the
// results played before its day.
func before[T any](rows []T, at *release, date func(T) string) []T {
	if at == nil {
		return rows
	}
	var out []T
	for _, r := range rows {
		if date(r) < at.date {
			out = append(out, r)
		}
	}
	return out
}

// upTo cuts a history at the release.
func (s *site) upTo(hist []histPt, at *release) []histPt {
	if at == nil {
		return hist
	}
	last := s.relIdx[at.id]
	for i, h := range hist {
		if h.rel > last {
			return hist[:i]
		}
	}
	return hist
}

// rankAt is the entity's place in the table of the release, from the place
// history that runs alongside its rating history; nil when it was not listed.
func (s *site) rankAt(kind int, id int64, at release) (*int64, error) {
	hist, err := s.history(kind, id)
	if err != nil {
		return nil, err
	}
	pl, err := s.q.places(kind, id)
	want := s.relIdx[at.id]
	for i, h := range hist[:min(len(hist), len(pl))] {
		if h.rel == want {
			n := int64(pl[i])
			return &n, err
		}
	}
	return nil, err
}

// asOfHead says which release the page shows, with the releases on either
// side and the way back to today's page.
func (s *site) asOfHead(href string, at *release) ui.Node {
	if at == nil {
		return nil
	}
	i := s.relIdx[at.id]
	items := []ui.Item{ui.Text(S.Common.AsOf.Lede(at.date) + " ")}
	if i > 0 {
		prev := &s.rels[i-1]
		items = append(items, link(asOfHref(href, prev), S.Common.AsOf.Prev(prev.date)), ui.Text(" · "))
	}
	next := &s.rels[i+1]
	if i+1 == len(s.rels)-1 {
		next = nil
	}
	if next != nil {
		items = append(items, link(asOfHref(href, next), S.Common.AsOf.Next(next.date)), ui.Text(" · "))
	}
	return head(append(items, link(href, S.Common.AsOf.Now()))...)
}

// pastLink is the way from today's page to the one a release earlier.
func (s *site) pastLink(href string) ui.Item {
	if len(s.rels) < 2 {
		return nil
	}
	prev := &s.rels[len(s.rels)-2]
	return line(link(asOfHref(href, prev), S.Common.AsOf.Earlier(prev.date)))
}

// fitRow is a player's own numbers at one release, as the fit wrote them.
type fitRow struct {
	theta, sigma float64
	vol          *float64
	games365     int64
	delta        *float64
}

func (r *fitRow) fields() []any { return []any{&r.theta, &r.sigma, &r.vol, &r.games365, &r.delta} }

// fitRowAt is the player's row at the release, or at the last release that
// lists them when at is nil.
func (st *store) fitRowAt(id int64, at *release) (fitRow, bool, error) {
	const cols = "select theta, sigma, vol, games_365, delta from player_ratings where player_id = ?"
	if at == nil {
		return one[fitRow](st.db, cols+" order by release_id desc limit 1", id)
	}
	return one[fitRow](st.db, cols+" and release_id = ?", id, at.id)
}

type teamRow struct {
	display float64
	change  *float64
}

func (r *teamRow) fields() []any { return []any{&r.display, &r.change} }

func (st *store) teamRowAt(id int64, at release) (teamRow, bool, error) {
	return one[teamRow](st.db, "select display, change from team_ratings where team_id = ? and release_id = ?", id, at.id)
}

// playerAt is the standing a player had at the release: the fit's row for
// it, the place in that release's table and the games played before it.
func (s *site) playerAt(p standing, f fitRow, ok bool, rank *int64, games int) standing {
	n := int64(games)
	p.games, p.rank, p.display, p.sigma, p.games365, p.change, p.prov = &n, rank, nil, nil, nil, nil, nil
	if ok {
		c := s.q.sc
		d, sd, prov := c.player(f.theta, 0), abs(c.bPlayer)*f.sigma, f.sigma > c.provSigma
		p.display, p.sigma, p.games365, p.change, p.prov = &d, &sd, &f.games365, f.delta, &prov
	}
	return p
}

// teamAt is the standing a team had at the release.
func teamAt(t standing, r teamRow, ok bool, rank *int64) standing {
	t.rank, t.display, t.change = rank, nil, nil
	if ok {
		t.display, t.change = &r.display, r.change
	}
	return t
}
