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

	s, err := open(*ratings, *web)
	if err != nil {
		log.Fatal(err)
	}
	var roots []string
	if *dev {
		roots = []string{"web", "."}
	}
	h := &handler{site: s, assets: kit.Assets(embedded, roots...)}
	log.Printf("serving %d teams and %d players from release %s on http://%s", s.nTeams, s.nPlayers, s.releaseDate, *addr)
	log.Fatal(http.ListenAndServe(*addr, noindex(webassets.Gzip(h.mux()))))
}

// open reads the databases and everything the pages share.
func open(ratings, web string) (*site, error) {
	q, err := openStore(ratings, web)
	if err != nil {
		return nil, err
	}
	s, err := loadSite(q)
	if err != nil {
		return nil, errors.Join(errors.New("read "+web+" (has webindex been run?)"), err)
	}
	return s, nil
}

func (h *handler) mux() *http.ServeMux {
	mux := http.NewServeMux()
	mux.Handle("/static/styles.css", h.assets.ServeStylesheet())
	mux.Handle("/static/menu.js", h.assets.ServeShared("/static/menu.js"))
	mux.Handle("/static/fonts/", h.assets.ServeFonts())
	mux.Handle("/static/", h.assets.FileServer())
	mux.Handle("/report.pdf", h.assets.ServeRoot("report.pdf", "application/pdf"))
	// The kit's menu asks who is signed in so it can offer a login link. Nobody
	// signs in here: an empty answer is one the menu cannot read, and it leaves
	// the entry out.
	mux.HandleFunc("/api/auth/me", func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusNoContent) })
	mux.Handle("/", h)
	return mux
}

// noindex keeps every page out of search engines until the site is launched.
func noindex(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Robots-Tag", "noindex, nofollow")
		next.ServeHTTP(w, r)
	})
}

type handler struct {
	site   *site
	assets *webassets.Assets
}

// page is a page's name and content; an entity page's name also ends the
// trail in the bar.
type page func(s *site, r *http.Request, segs []string) (string, []ui.Item, error)

// listings are the paged tables under their section's address.
var listings = map[string]func(*site, int) (string, []ui.Item, error){
	"players":     (*site).playerTable,
	"tournaments": (*site).tournamentTable,
	"anomaly":     (*site).anomalyTable,
}

// entities are the pages of one team, player or tournament.
var entities = map[string]func(*site, int64) (string, []ui.Item, error){
	"players":     (*site).playerPage,
	"teams":       (*site).teamPage,
	"tournaments": (*site).tournamentPage,
}

// singles are the pages with an address of one segment and no listing.
var singles = map[string]page{
	"releases": func(s *site, _ *http.Request, _ []string) (string, []ui.Item, error) { return s.releasesPage() },
	"method": func(*site, *http.Request, []string) (string, []ui.Item, error) {
		title, body := methodPage()
		return title, body, nil
	},
	"search": func(s *site, r *http.Request, _ []string) (string, []ui.Item, error) {
		return s.searchPage(r.URL.Query().Get("q"))
	},
}

// sectionOf is the part of the site an address belongs to, for its pill and
// its crumb.
var sectionOf = map[string]string{"page": "teams", "players": "players", "teams": "teams", "tournaments": "tournaments",
	"releases": "releases", "method": "method", "anomaly": "anomaly", "search": "search"}

// route finds the page at an address, by its segments, so a page answers with
// or without its trailing slash.
func route(segs []string) page {
	switch {
	case len(segs) == 0:
		return home
	case len(segs) == 1 && listings[segs[0]] != nil:
		return listed(segs[0], -1)
	case len(segs) == 1:
		return singles[segs[0]]
	case len(segs) == 2 && segs[0] == "page":
		return func(s *site, _ *http.Request, segs []string) (string, []ui.Item, error) {
			return s.teamTable(pageNo(segs[1]))
		}
	case len(segs) == 2:
		return entity(segs[0])
	case len(segs) == 3 && segs[1] == "page" && listings[segs[0]] != nil:
		return listed(segs[0], 2)
	}
	return nil
}

// home is the first page of the team table under the site's own name.
func home(s *site, _ *http.Request, _ []string) (string, []ui.Item, error) {
	_, body, err := s.teamTable(1)
	return S.Common.Brand(), body, err
}

// listed is a listing at the page number in segment at, or its first page.
func listed(name string, at int) page {
	return func(s *site, _ *http.Request, segs []string) (string, []ui.Item, error) {
		n := 1
		if at >= 0 {
			n = pageNo(segs[at])
		}
		return listings[name](s, n)
	}
}

func entity(kind string) page {
	f := entities[kind]
	if f == nil {
		return nil
	}
	return func(s *site, _ *http.Request, segs []string) (string, []ui.Item, error) {
		id, err := strconv.ParseInt(segs[1], decimal, idBits)
		if err != nil {
			return "", nil, errNotFound
		}
		return f(s, id)
	}
}

// idBits is the width of an id in an address.
const idBits = 64

// pageNo is a page number from the address; anything else is the first page.
func pageNo(seg string) int {
	if v, err := strconv.Atoi(seg); err == nil {
		return v
	}
	return 1
}

func (h *handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	segs := strings.FieldsFunc(r.URL.Path, func(c rune) bool { return c == '/' })
	section := "teams"
	if len(segs) > 0 {
		section = sectionOf[segs[0]]
	}
	var title string
	var body []ui.Item
	err := errNotFound
	if p := route(segs); p != nil {
		title, body, err = p(h.site, r, segs)
	}
	leaf := ""
	if len(segs) == 2 && segs[0] != "page" {
		leaf = title
	}
	h.respond(w, r, err, title, section, leaf, body)
}

// respond sends the page, or the page that says why there is none.
func (h *handler) respond(w http.ResponseWriter, r *http.Request, err error, title, section, leaf string, body []ui.Item) {
	switch {
	case errors.Is(err, errNotFound):
		title := S.Common.NotFound.Title()
		h.send(w, http.StatusNotFound, title, "", title, head(ui.Text(S.Common.NotFound.Lede())))
	case err != nil:
		log.Printf("%s: %v", r.URL.Path, err)
		title := S.Common.Failed.Title()
		h.send(w, http.StatusInternalServerError, title, "", title, head(ui.Text(S.Common.Failed.Lede())))
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
