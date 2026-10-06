package main

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"fmt"
	"strings"
	"sync"

	"modernc.org/sqlite"
)

// openRatings opens out/ratings.db read-only with the serving index attached as
// `x`, so a page joins names onto ratings in a single query. Every connection
// in the pool gets the attachment from the hook, not just the first one.
func openRatings(ratings, web string) (*sql.DB, error) {
	attachOnce.Do(func() {
		sqlite.RegisterConnectionHook(func(conn sqlite.ExecQuerierContext, dsn string) error {
			if attachPath == "" {
				return nil
			}
			_, err := conn.ExecContext(context.Background(), "ATTACH DATABASE ? AS x", []driver.NamedValue{{Ordinal: 1, Value: "file:" + attachPath + "?mode=ro"}})
			return err
		})
	})
	attachPath = web
	pragmas := []string{
		"mode=ro",
		"_pragma=query_only(1)",
		"_pragma=cache_size(-65536)",
		"_pragma=mmap_size(1073741824)",
	}
	db, err := sql.Open("sqlite", "file:"+ratings+"?"+strings.Join(pragmas, "&"))
	if err != nil {
		return nil, err
	}
	if err := db.Ping(); err != nil {
		db.Close()
		return nil, fmt.Errorf("open %s: %w", ratings, err)
	}
	return db, nil
}

var (
	attachOnce sync.Once
	attachPath string
)

// columns is the set of column names a table has, so a page survives the
// schema growing a column under it. A table that is not there has none.
func columns(db *sql.DB, table string) map[string]bool {
	out := map[string]bool{}
	rows, err := db.Query("select name from pragma_table_info(?)", table)
	if err != nil {
		return out
	}
	defer rows.Close()
	for rows.Next() {
		var name string
		if rows.Scan(&name) == nil {
			out[name] = true
		}
	}
	return out
}

func tableExists(db *sql.DB, table string) bool { return len(columns(db, table)) > 0 }

// col is `name` when the table has it and the literal NULL when it does not, so
// one projection serves both schemas.
func col(have map[string]bool, name string) string {
	if have[name] {
		return name
	}
	return "NULL"
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
