package main

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"

	"modernc.org/sqlite"
)

// indexVersion is the web.db this server reads: schema.rs's VERSION on the
// Rust side, bumped with it.
const indexVersion = "16"

// The kinds of place_hist, as schema.rs numbers them.
const (
	kindPlayer = 0
	kindTeam   = 1
)

// store is every query the site makes. It owns the SQL against out/ratings.db
// and the serving index attached to it as `x`, and hands pages typed rows with
// every number already on the published scale.
type store struct {
	db *sql.DB
	sc scale
	// achieve is set when the fit published the achievement rating.
	achieve bool
	// teamSteps is set when team_events splits a step into placement and pull.
	teamSteps bool
	// cleanFrom is the first day of the clean test window; empty for none.
	cleanFrom string
	// s2 is set for the published team engine, which carries no team
	// uncertainty: pages show rated games where they would show a ±.
	s2 bool
}

// scale maps the model's numbers onto the published one. The index stores the
// affine maps in its meta table.
type scale struct {
	aTeam, bTeam, aPlayer, bPlayer float64
	// playerPhi is how much of the in-person effect a player's number carries.
	playerPhi float64
	// provSigma is the uncertainty above which a rating is provisional.
	provSigma float64
	// rankSigmas is how many sigmas below the rating the player table ranks
	// by; above 0 every player has a place, provisional or not.
	rankSigmas float64
}

func (c scale) team(mu float64) float64 { return c.aTeam + c.bTeam*mu }

// player is the published number of a player of strength theta with the
// in-person effect phi.
func (c scale) player(theta, phi float64) float64 {
	return c.aPlayer + c.bPlayer*(theta+c.playerPhi*phi)
}

// openStore opens out/ratings.db read-only with the serving index attached,
// and refuses an index built for another version of this server.
func openStore(ratings, web string) (*store, error) {
	db, err := openRatings(ratings, web)
	if err != nil {
		return nil, err
	}
	st := &store{db: db}
	if v, _ := st.meta("index_version"); v != indexVersion {
		db.Close()
		return nil, fmt.Errorf("%s is index version %q, this server reads %q: run webindex", web, v, indexVersion)
	}
	return st, st.readMeta()
}

func (st *store) readMeta() error {
	var err error
	num := func(key string) float64 {
		v, e := st.meta(key)
		f, pe := strconv.ParseFloat(v, 64)
		err = errors.Join(err, e, pe)
		return f
	}
	st.sc = scale{aTeam: num("a_team"), bTeam: num("b_team"), aPlayer: num("a_player"), bPlayer: num("b_player"),
		playerPhi: num("player_phi"), provSigma: num("provisional_sigma"), rankSigmas: num("rank_sigmas")}
	st.achieve, st.teamSteps, st.s2 = num("achieve") != 0, num("team_steps") != 0, num("s2") != 0
	st.cleanFrom, _ = st.meta("clean_from")
	return err
}

func (st *store) meta(key string) (string, error) {
	var v string
	err := st.db.QueryRow("select value from x.meta where key = ?", key).Scan(&v)
	return v, err
}

func (st *store) metaInt(key string) (int, error) {
	v, err := st.meta(key)
	if err != nil {
		return 0, err
	}
	return strconv.Atoi(v)
}

// openRatings opens out/ratings.db read-only with the serving index attached as
// `x`, so a page joins names onto ratings in a single query. Every connection
// in the pool gets the attachment from the hook, not just the first one; a
// connection to any other file gets none.
func openRatings(ratings, web string) (*sql.DB, error) {
	attachOnce.Do(func() { sqlite.RegisterConnectionHook(attach) })
	pragmas := []string{
		"mode=ro",
		"_pragma=query_only(1)",
		"_pragma=cache_size(-65536)",
		"_pragma=mmap_size(1073741824)",
	}
	dsn := "file:" + ratings + "?" + strings.Join(pragmas, "&")
	attachments.Store(dsn, web)
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	if err := db.Ping(); err != nil {
		db.Close()
		return nil, fmt.Errorf("open %s: %w", ratings, err)
	}
	return db, nil
}

func attach(conn sqlite.ExecQuerierContext, dsn string) error {
	web, ok := attachments.Load(dsn)
	if !ok {
		return nil
	}
	arg := []driver.NamedValue{{Ordinal: 1, Value: "file:" + web.(string) + "?mode=ro"}}
	_, err := conn.ExecContext(context.Background(), "ATTACH DATABASE ? AS x", arg)
	return err
}

var (
	attachOnce sync.Once
	// attachments is the index each ratings DSN opened here attaches.
	attachments sync.Map
)

// fielded is a row type that names the places its columns scan into, in the
// order its query selects them.
type fielded[T any] interface {
	*T
	fields() []any
}

// all runs a query and scans every row into a T.
func all[T any, P fielded[T]](db *sql.DB, query string, args ...any) ([]T, error) {
	rows, err := db.Query(query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []T
	for rows.Next() {
		var v T
		if err := rows.Scan(P(&v).fields()...); err != nil {
			return nil, err
		}
		out = append(out, v)
	}
	return out, rows.Err()
}

// one runs a query for a single row; ok is false when there is none.
func one[T any, P fielded[T]](db *sql.DB, query string, args ...any) (v T, ok bool, err error) {
	err = db.QueryRow(query, args...).Scan(P(&v).fields()...)
	if errors.Is(err, sql.ErrNoRows) {
		return v, false, nil
	}
	return v, err == nil, err
}

// str, num and count read a nullable column the way the pages want it: absent
// is the zero value.
func str(p *string) string {
	if p == nil {
		return ""
	}
	return *p
}

func num(p *float64) float64 {
	if p == nil {
		return 0
	}
	return *p
}

func count(p *int64) int64 {
	if p == nil {
		return 0
	}
	return *p
}

// mapped is f of a nullable value, nil when there is none.
func mapped(p *float64, f func(float64) float64) *float64 {
	if p == nil {
		return nil
	}
	v := f(*p)
	return &v
}
