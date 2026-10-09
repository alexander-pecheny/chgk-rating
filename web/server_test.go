package main

import (
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"

	"pecheny.me/dopeuikit/kit"
)

func TestFormat(t *testing.T) {
	for _, c := range []struct{ got, want string }{
		{disp(-0.4), "0"},
		{disp(2.5), "3"},
		{signed(3), "+3"},
		{signed(-3), "−3"},
		{signed(0.2), "0"},
		{signed1(-1.25), "−1.2"},
		{placeOf(2), "2"},
		{placeOf(2.5), "2.5"},
		{spaced(1234567), "1 234 567"},
		{w2(0.9), "0,90"},
		{ftsQuery(`Иван "Петров"-Водкин`), `"Иван"* "Петров"* "Водкин"*`},
	} {
		if c.got != c.want {
			t.Errorf("got %q, want %q", c.got, c.want)
		}
	}
}

func TestRankDesc(t *testing.T) {
	v := func(x float64) *float64 { return &x }
	got := rankDesc([]*float64{v(10), v(20), v(10), nil})
	if *got[0] != 2.5 || *got[1] != 1 || *got[2] != 2.5 || got[3] != nil {
		t.Errorf("ranks %v %v %v %v", *got[0], *got[1], *got[2], got[3])
	}
}

// TestPages renders every kind of page against the real databases, when the
// checkout has them: a page the vocabulary rejects fails here, not in front of
// a reader. RATINGS and WEB override the default paths.
func TestPages(t *testing.T) {
	ratings, web := env("RATINGS", "../out/ratings.db"), env("WEB", "../out/web.db")
	if _, err := os.Stat(web); err != nil {
		t.Skip("no serving index; run webindex first")
	}
	s, err := open(ratings, web)
	if err != nil {
		t.Fatal(err)
	}
	db := s.q.db
	h := &handler{site: s, assets: kit.Assets(embedded)}
	var team, player, tour int64
	_ = db.QueryRow("select team_id from x.team_rank where rank = 1").Scan(&team)
	_ = db.QueryRow("select player_id from x.player_rank where rank = 1").Scan(&player)
	_ = db.QueryRow("select tournament_id from x.venue group by field_id having count(*) > 1 limit 1").Scan(&tour)
	for _, c := range []struct {
		path   string
		status int
	}{
		{"/", 200}, {"/page/2/", 200}, {"/players/", 200}, {"/players/page/3", 200},
		{"/tournaments/", 200}, {"/releases/", 200}, {"/anomaly/", 200}, {"/method/", 200},
		{"/search/?q=%D0%B0", 200}, {"/search/", 200},
		{"/teams/" + itoa(team) + "/", 200}, {"/players/" + itoa(player) + "/", 200},
		{"/tournaments/" + itoa(tour) + "/", 200},
		{"/teams/0/", 404}, {"/players/x/", 404}, {"/nowhere/", 404},
	} {
		w := httptest.NewRecorder()
		h.ServeHTTP(w, httptest.NewRequest(http.MethodGet, c.path, nil))
		if w.Code != c.status {
			t.Errorf("%s: status %d, want %d", c.path, w.Code, c.status)
		}
		if body := w.Body.String(); !strings.Contains(body, `game-host-top`) {
			t.Errorf("%s: no site header in the page", c.path)
		}
	}
}

func env(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}

func TestNoindex(t *testing.T) {
	rec := httptest.NewRecorder()
	noindex(http.NotFoundHandler()).ServeHTTP(rec, httptest.NewRequest("GET", "/", nil))
	if got := rec.Header().Get("X-Robots-Tag"); got != "noindex, nofollow" {
		t.Fatalf("X-Robots-Tag = %q", got)
	}
}
