package main

import "errors"

// pageSize is the length of every listing page.
const pageSize = 100

// chartPoints caps the points a history chart draws.
const chartPoints = 200

// site is everything read once at start-up and shared by every request.
type site struct {
	q           *store
	rels        []release
	relIdx      map[int64]int
	releaseDate string
	nTeams      int
	nPlayers    int
	nAnomaly    int
	// method is the post at /method/, as HTML.
	method string
	// model is the version of the rules that made the numbers; empty for a
	// fit from before versions.
	model string
}

func loadSite(q *store) (*site, error) {
	rels, err := q.releases()
	s := &site{q: q, rels: rels, relIdx: map[int64]int{}}
	for i, r := range rels {
		s.relIdx[r.id] = i
	}
	date, e1 := q.meta("release_date")
	teams, e2 := q.metaInt("n_team_rank")
	players, e3 := q.metaInt("n_player_rank")
	anomaly, e4 := q.anomalyCount()
	method, e5 := renderMethod()
	model, _ := q.meta("model_version")
	s.releaseDate, s.nTeams, s.nPlayers, s.nAnomaly, s.method, s.model = date, teams, players, anomaly, method, model
	return s, errors.Join(err, e1, e2, e3, e4, e5)
}

type tick struct {
	x     float64
	label string
}

// ticks are dated marks across the releases a chart covers, each a month.
func (s *site) ticks(from, to, marks int) []tick {
	const (
		month = len("2006-01")
		half  = 0.5
	)
	var v []tick
	for k := 0; k < marks; k++ {
		i := float64(from) + float64(to-from)*float64(k)/float64(marks-1)
		label := prefix(s.rels[int(i+half)].date, month)
		if len(v) > 0 && v[len(v)-1].label == label {
			continue
		}
		v = append(v, tick{i, label})
	}
	return v
}
