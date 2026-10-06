// Command serve is the rating site: rating2.pecheny.me. It reads the fit's
// out/ratings.db with the serving index out/web.db attached, both built by the
// Rust side (`fit`, then `webindex`), and renders every page per request
// through DopeUIKit's kit. Nothing here computes a rating.
//
//	serve [--ratings out/ratings.db] [--web out/web.db] [--addr 127.0.0.1:8080] [--dev]
package main

import (
	"embed"
	"errors"
	"flag"
	"log"
	"net/http"
	"strconv"
	"strings"

	"pecheny.me/dopecore/webassets"
	"pecheny.me/dopeuikit/kit"

	"ratingweb/ui"
)

//go:embed static
var embedded embed.FS

func main() {
	ratings := flag.String("ratings", "out/ratings.db", "the fit's output")
	web := flag.String("web", "out/web.db", "the serving index webindex builds")
	addr := flag.String("addr", "127.0.0.1:8080", "where to listen")
	dev := flag.Bool("dev", false, "read web/static from disk on every request, for editing the CSS and JS")
	flag.Parse()

	db, err := openRatings(*ratings, *web)
	if err != nil {
		log.Fatal(err)
	}
	s, err := loadSite(db)
	if err != nil {
		log.Fatalf("read %s: %v (has webindex been run?)", *web, err)
	}
	var roots []string
	if *dev {
		roots = []string{"web", "."}
	}
	assets := kit.Assets(embedded, roots...)
	h := &handler{site: s, assets: assets}

	mux := http.NewServeMux()
	mux.Handle("/static/styles.css", assets.ServeStylesheet())
	mux.Handle("/static/menu.js", assets.ServeShared("/static/menu.js"))
	mux.Handle("/static/fonts/", assets.ServeFonts())
	mux.Handle("/static/", assets.FileServer())
	mux.Handle("/report.pdf", assets.ServeRoot("report.pdf", "application/pdf"))
	// The kit's menu asks who is signed in so it can offer a login link. Nobody
	// signs in here: an empty answer is one the menu cannot read, and it leaves
	// the entry out.
	mux.HandleFunc("/api/auth/me", func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusNoContent) })
	mux.Handle("/", h)

	log.Printf("serving %d teams and %d players from release %s on http://%s", s.nTeams, s.nPlayers, s.releaseDate, *addr)
	log.Fatal(http.ListenAndServe(*addr, webassets.Gzip(mux)))
}

type handler struct {
	site   *site
	assets *webassets.Assets
}

// ServeHTTP routes by path segment, so a page answers with or without its
// trailing slash.
func (h *handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	var segs []string
	for _, p := range strings.Split(r.URL.Path, "/") {
		if p != "" {
			segs = append(segs, p)
		}
	}
	n := func(i int) int {
		if i < len(segs) {
			if v, err := strconv.Atoi(segs[i]); err == nil {
				return v
			}
		}
		return 1
	}
	id := func(i int) (int64, bool) {
		if i < len(segs) {
			if v, err := strconv.ParseInt(segs[i], 10, 64); err == nil {
				return v, true
			}
		}
		return 0, false
	}
	s := h.site
	var title string
	var body []ui.Item
	var err error
	// section is the part of the site the page belongs to, for its pill and
	// its crumb; an entity page also ends its trail with its own name.
	section, entity := "", false
	if len(segs) > 0 {
		section = map[string]string{"page": "teams", "players": "players", "teams": "teams", "tournaments": "tournaments",
			"releases": "releases", "method": "method", "anomaly": "anomaly", "search": "search"}[segs[0]]
		entity = len(segs) == 2 && segs[0] != "page"
	} else {
		section = "teams"
	}
	switch path := strings.Join(segs, "/"); {
	case len(segs) == 0:
		_, body, err = s.teamTable(1)
		title = S.Common.Brand()
	case len(segs) == 2 && segs[0] == "page":
		title, body, err = s.teamTable(n(1))
	case path == "players":
		title, body, err = s.playerTable(1)
	case len(segs) == 3 && segs[0] == "players" && segs[1] == "page":
		title, body, err = s.playerTable(n(2))
	case path == "tournaments":
		title, body, err = s.tournamentTable(1)
	case len(segs) == 3 && segs[0] == "tournaments" && segs[1] == "page":
		title, body, err = s.tournamentTable(n(2))
	case path == "anomaly":
		title, body, err = s.anomalyTable(1)
	case len(segs) == 3 && segs[0] == "anomaly" && segs[1] == "page":
		title, body, err = s.anomalyTable(n(2))
	case path == "releases":
		title, body, err = s.releasesPage()
	case path == "method":
		title, body = methodPage()
	case path == "search":
		title, body, err = s.searchPage(r.URL.Query().Get("q"))
	case len(segs) == 2 && segs[0] == "players":
		if v, ok := id(1); ok {
			title, body, err = s.playerPage(v)
		} else {
			err = errNotFound
		}
	case len(segs) == 2 && segs[0] == "teams":
		if v, ok := id(1); ok {
			title, body, err = s.teamPage(v)
		} else {
			err = errNotFound
		}
	case len(segs) == 2 && segs[0] == "tournaments":
		if v, ok := id(1); ok {
			title, body, err = s.tournamentPage(v)
		} else {
			err = errNotFound
		}
	default:
		err = errNotFound
	}
	leaf := ""
	if entity {
		leaf = title
	}
	switch {
	case errors.Is(err, errNotFound):
		h.send(w, http.StatusNotFound, S.Common.NotFound.Title(), "", S.Common.NotFound.Title(), head(S.Common.NotFound.Title(), ui.Text(S.Common.NotFound.Lede())))
	case err != nil:
		log.Printf("%s: %v", r.URL.Path, err)
		h.send(w, http.StatusInternalServerError, S.Common.Failed.Title(), "", S.Common.Failed.Title(), head(S.Common.Failed.Title(), ui.Text(S.Common.Failed.Lede())))
	default:
		h.send(w, http.StatusOK, title, section, leaf, body...)
	}
}

func (h *handler) send(w http.ResponseWriter, status int, title, section, leaf string, body ...ui.Item) {
	page, err := ui.Render(layout(title, section, leaf, body...))
	if err != nil {
		// A tree the vocabulary rejects is a bug in this server, not in the data.
		log.Printf("render %q: %v", title, err)
		http.Error(w, "render failed", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.WriteHeader(status)
	_, _ = w.Write(h.assets.VersionRefs(page))
}
