package main

import (
	"database/sql"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"pecheny.me/dopeuikit/kit"
)

// fixtureRatings is the part of a fit's out/ratings.db the site reads, with
// one in-person tournament of two teams and three players. No achievement
// tables, so the field shows the step of the rating and its parts.
const fixtureRatings = `
CREATE TABLE player_ratings (release_id INTEGER, player_id INTEGER, theta REAL, sigma REAL);
CREATE TABLE team_ratings (release_id INTEGER, team_id INTEGER, mu REAL, sigma REAL);
CREATE TABLE player_events (tournament_id INTEGER, player_id INTEGER, theta_before REAL, theta_after REAL,
                            phi_before REAL, sigma_before REAL);
CREATE TABLE team_events (tournament_id INTEGER, team_id INTEGER, date TEXT, forecast REAL, forecast_place REAL,
                          actual_place REAL, mu_before REAL, mu_after REAL, rated INTEGER, s_r REAL,
                          d_place REAL, d_pull REAL);
CREATE TABLE question_params (tournament_id INTEGER, q INTEGER, b REAL);
INSERT INTO player_ratings VALUES (1, 100, 0.40, 0.1), (2, 100, 0.50, 0.1), (1, 101, 0.20, 0.5), (2, 101, 0.25, 0.5);
INSERT INTO team_ratings VALUES (1, 10, 9.0, 0.1), (2, 10, 9.05, 0.1), (1, 11, 8.0, 0.1), (2, 11, 7.9, 0.1);
INSERT INTO player_events VALUES (500, 100, 0.40, 0.50, 0.10, 0.1), (500, 101, 0.20, 0.25, NULL, 0.5),
                                 (500, 102, 0.10, 0.08, 0.0, 0.1);
INSERT INTO team_events VALUES (500, 10, '2026-09-01', 2.0, 2, 1, 9.0, 9.05, 1, 9.5, 0.03, 0.02),
                               (500, 11, '2026-09-01', 1.0, 1, 2, 8.0, 7.9, 0, 8.5, -0.06, -0.04);
INSERT INTO question_params VALUES (500, 1, -1.0), (500, 2, 0.5), (500, 3, 2.0);
`

// fixtureIndex is the web.db webindex would build for it.
const fixtureIndex = `
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE release_ (id INTEGER PRIMARY KEY, date TEXT, shift REAL, shift_cum REAL, n_players INTEGER, n_teams INTEGER);
CREATE TABLE player (id INTEGER PRIMARY KEY, name TEXT, games INTEGER, last_date TEXT,
                     display REAL, sigma REAL, games_365 INTEGER, provisional INTEGER);
CREATE TABLE team (id INTEGER PRIMARY KEY, name TEXT, town TEXT, games INTEGER, last_date TEXT,
                   display REAL, sigma REAL, provisional INTEGER);
CREATE TABLE tournament (id INTEGER PRIMARY KEY, name TEXT, date TEXT, type TEXT, weight REAL,
                         n_teams INTEGER, n_q INTEGER, n_obs INTEGER, ll_in REAL, ll_cf REAL,
                         sp_forecast REAL, sp_old REAL, truedl REAL);
CREATE TABLE player_game (player_id INTEGER, tournament_id INTEGER, team_id INTEGER, position REAL, score INTEGER);
CREATE TABLE result_ (tournament_id INTEGER, team_id INTEGER, score INTEGER, name TEXT);
CREATE TABLE player_rank (rank INTEGER PRIMARY KEY, player_id INTEGER, display REAL, sigma REAL,
                          games INTEGER, change REAL, provisional INTEGER);
CREATE TABLE team_rank (rank INTEGER PRIMARY KEY, team_id INTEGER, display REAL, sigma REAL,
                        evidence REAL, change REAL, provisional INTEGER);
CREATE TABLE anomaly (rank INTEGER PRIMARY KEY, player_id INTEGER, k REAL, k_f REAL, k_m REAL,
                      f_gap REAL, f_on INTEGER, f_ip INTEGER, m_gap REAL, m_on INTEGER, m_ip INTEGER);
CREATE TABLE venue (tournament_id INTEGER PRIMARY KEY, field_id INTEGER);
CREATE TABLE field_score (tournament_id INTEGER, team_id INTEGER, score INTEGER, PRIMARY KEY (tournament_id, team_id)) WITHOUT ROWID;
CREATE TABLE base_roster (team_id INTEGER, player_id INTEGER, name TEXT, date_added TEXT);
CREATE TABLE place_hist (kind INTEGER, id INTEGER, places BLOB, PRIMARY KEY (kind, id)) WITHOUT ROWID;
CREATE VIRTUAL TABLE player_fts USING fts5(name);
CREATE VIRTUAL TABLE team_fts USING fts5(name, town, aliases);
INSERT INTO meta VALUES ('index_version', '` + indexVersion + `'), ('release_id', '2'), ('release_date', '2026-09-03'),
  ('n_player_rank', '2'), ('n_team_rank', '2'), ('a_team', '0'), ('b_team', '1000'), ('a_player', '0'),
  ('b_player', '1000'), ('player_phi', '1'), ('provisional_sigma', '0.3'), ('rank_sigmas', '0'), ('s2', '1'), ('achieve', '0'),
  ('team_steps', '1');
INSERT INTO release_ VALUES (1, '2026-08-27', 0, 0, 2, 2), (2, '2026-09-03', 0.01, 0.01, 2, 2);
INSERT INTO player VALUES (100, 'Анна Иванова', 1, '2026-09-01', 500, 100, 1, 0),
                          (101, 'Борис Петров', 1, '2026-09-01', 250, 500, 1, 1),
                          (102, 'Вера Сидорова', 1, '2026-09-01', NULL, NULL, NULL, NULL);
INSERT INTO team VALUES (10, 'Альфа', 'Москва', 1, '2026-09-01', 9050, 106.4, 0),
                        (11, 'Бета', '', 1, '2026-09-01', 7900, 106.4, 0);
INSERT INTO tournament VALUES (500, 'Кубок', '2026-09-01', 'in-person', 1.0, 2, 36, 72, 0.5, 0.55, 0.8, 0.6, 4.2);
INSERT INTO player_game VALUES (100, 500, 10, 1, 30), (101, 500, 10, 1, 30), (102, 500, 11, 2, 25);
INSERT INTO result_ VALUES (500, 10, 30, 'Альфа'), (500, 11, 25, 'Бета-2');
INSERT INTO player_rank VALUES (1, 100, 500, 100, 1, 100, 0), (2, 101, 250, 500, 1, 50, 1);
INSERT INTO team_rank VALUES (1, 10, 9050, 106.4, 1, 50, 0), (2, 11, 7900, 106.4, 1, -100, 0);
INSERT INTO base_roster VALUES (10, 100, 'Анна Иванова', '2026-01-01');
INSERT INTO place_hist VALUES (1, 10, X'0100000001000000'), (0, 100, X'0100000001000000');
INSERT INTO player_fts(rowid, name) VALUES (100, 'Анна Иванова'), (101, 'Борис Петров'), (102, 'Вера Сидорова');
INSERT INTO team_fts(rowid, name, town, aliases) VALUES (10, 'Альфа', 'Москва', ''), (11, 'Бета', '', 'Бета-2');
`

// fixture writes the two databases into a temporary directory.
func fixture(t *testing.T, index string) (ratings, web string) {
	dir := t.TempDir()
	ratings, web = filepath.Join(dir, "ratings.db"), filepath.Join(dir, "web.db")
	for path, script := range map[string]string{ratings: fixtureRatings, web: index} {
		db, err := sql.Open("sqlite", "file:"+path)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := db.Exec(script); err != nil {
			t.Fatalf("%s: %v", path, err)
		}
		db.Close()
	}
	return ratings, web
}

// TestFixturePages renders each kind of page against the fixture and checks
// the numbers the store puts on the published scale.
func TestFixturePages(t *testing.T) {
	s, err := open(fixture(t, fixtureIndex))
	if err != nil {
		t.Fatal(err)
	}
	h := &handler{site: s, assets: kit.Assets(embedded)}
	for _, c := range []struct {
		path   string
		status int
		want   []string
	}{
		{"/", 200, []string{`href="/teams/10/"`, "Альфа", "9050", "Москва"}},
		{"/players/", 200, []string{"Анна Иванова", "Борис Петров"}},
		{"/teams/10/", 200, []string{"Москва", "№1", "Анна Иванова", "Кубок"}},
		// Before and after at an in-person table, theta + phi: 500 and 600.
		{"/players/100/", 200, []string{"Анна Иванова", ">500<", ">600<", "+100"}},
		// The step of Альфа is +50: +30 by place and +20 of pull. Бета entered
		// as Бета-2 and its result did not count.
		{"/tournaments/500/", 200, []string{">9000<", ">9050<", "+50", "+30", "+20", "Бета-2", ">9500<"}},
		{"/search/?q=%D0%B1", 200, []string{`href="/teams/11/"`, `href="/players/101/"`}},
		{"/releases/", 200, []string{"2026-09-03"}},
		{"/anomaly/", 200, nil},
		{"/tournaments/1/", 404, nil},
		{"/teams/x/", 404, nil},
	} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, httptest.NewRequest(http.MethodGet, c.path, nil))
		if w.Code != c.status {
			t.Errorf("%s: status %d, want %d", c.path, w.Code, c.status)
		}
		for _, want := range c.want {
			if !strings.Contains(w.Body.String(), want) {
				t.Errorf("%s: no %q in the page", c.path, want)
			}
		}
	}
}

// TestIndexVersion refuses an index built for another server.
func TestIndexVersion(t *testing.T) {
	old := strings.Replace(fixtureIndex, "'"+indexVersion+"'", "'0'", 1)
	if _, err := open(fixture(t, old)); err == nil || !strings.Contains(err.Error(), "index version") {
		t.Errorf("an old index opened: %v", err)
	}
}
