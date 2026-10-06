package main

import (
	"database/sql"
	"strconv"
)

// pageSize is the length of every listing page.
const pageSize = 100

// chartPoints caps the points a history chart draws.
const chartPoints = 200

// display maps the model's numbers onto the published scale. The index stores
// the affine maps in its meta table.
type display struct {
	aTeam, bTeam, aPlayer, bPlayer float64
	// provSigma is the uncertainty above which a rating is provisional.
	provSigma float64
	// s2 is set for the published team engine, which carries no team
	// uncertainty: pages show rated games where they would show a ±.
	s2 bool
}

type release struct {
	id   int64
	date string
}

// site is everything read once at start-up and shared by every request.
type site struct {
	db          *sql.DB
	d           display
	rels        []release
	relIdx      map[int64]int
	releaseDate string
	nTeams      int
	nPlayers    int
	nAnomaly    int
}

func loadSite(db *sql.DB) (*site, error) {
	s := &site{db: db, relIdx: map[int64]int{}}
	s.d = display{
		aTeam:     s.meta("a_team"),
		bTeam:     s.meta("b_team"),
		aPlayer:   s.meta("a_player"),
		bPlayer:   s.meta("b_player"),
		provSigma: 0.3,
		s2:        s.meta("s2") != 0,
	}
	rows, err := db.Query("select id, date from x.release_ order by date, id")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() {
		var r release
		if err := rows.Scan(&r.id, &r.date); err != nil {
			return nil, err
		}
		s.relIdx[r.id] = len(s.rels)
		s.rels = append(s.rels, r)
	}
	_ = db.QueryRow("select value from x.meta where key = 'release_date'").Scan(&s.releaseDate)
	s.nTeams = int(s.meta("n_team_rank"))
	s.nPlayers = int(s.meta("n_player_rank"))
	_ = db.QueryRow("select count(*) from x.anomaly").Scan(&s.nAnomaly)
	return s, nil
}

func (s *site) meta(key string) float64 {
	var v string
	if s.db.QueryRow("select value from x.meta where key = ?", key).Scan(&v) != nil {
		return 0
	}
	f, _ := strconv.ParseFloat(v, 64)
	return f
}

type tick struct {
	x     float64
	label string
}

// ticks are four dated marks across the releases a chart covers.
func (s *site) ticks(from, to int) []tick {
	var v []tick
	for k := 0; k < 4; k++ {
		i := float64(from) + float64(to-from)*float64(k)/3
		label := s.rels[int(i+0.5)].date
		if len(label) > 7 {
			label = label[:7]
		}
		if len(v) > 0 && v[len(v)-1].label == label {
			continue
		}
		v = append(v, tick{i, label})
	}
	return v
}
