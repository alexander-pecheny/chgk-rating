package main

import (
	"math"

	sitestrings "ratingweb/i18nstrings"
	"ratingweb/ui"
)

// S is the Catalog every page reads its words from.
var S = sitestrings.Default

// sections are the parts of the site, in the order the pill strip under the
// header lists them, each with the address of its first page.
var sections = []struct{ key, href string }{
	{"teams", "/"}, {"players", "/players/"}, {"tournaments", "/tournaments/"},
	{"releases", "/releases/"}, {"method", "/method/"},
}

var sectionNames = map[string]func() string{
	"teams":       S.Common.Nav.Teams,
	"players":     S.Common.Nav.Players,
	"tournaments": S.Common.Nav.Tournaments,
	"releases":    S.Common.Nav.Releases,
	"method":      S.Common.Nav.Method,
	"anomaly":     S.Anomaly.Title,
	"search":      S.Search.Title,
}

func sectionName(key string) string {
	if f, ok := sectionNames[key]; ok {
		return f()
	}
	return ""
}

// orBrand is a section's name, or the site's on a page outside every section.
func orBrand(name string) string {
	if name == "" {
		return S.Common.Brand()
	}
	return name
}

func sectionHref(key string) string {
	for _, x := range sections {
		if x.key == key {
			return x.href
		}
	}
	return "/" + key + "/"
}

// layout is the page shell, dope's public page: the bar with the trail from
// home to here and the search box, the strip of the site's sections, the
// page's own content, and the footer. section is the part of the site the page
// belongs to; leaf, when set, is the page's own name at the end of the trail.
func layout(title, section, leaf string, body ...ui.Item) *ui.Doc {
	if title != S.Common.Brand() {
		title = S.Common.Title(title)
	}
	items := []ui.Item{
		ui.Title(title), ui.PageSite, ui.Classicscripts("site.js"),
		ui.Sitebar(ui.Crumbs(trail(section, leaf)...), ui.Tablinks(append([]ui.Item{ui.Label(orBrand(sectionName(section)))}, pills(section, leaf)...)...)),
	}
	content := append(body, ui.Sitefoot(ui.Text(S.Common.Foot.Note())))
	items = append(items, ui.Sitebody(content...))
	return &ui.Doc{Nodes: []ui.Node{ui.Page(items...)}}
}

// trail is the bar's crumbs: home, the section, and the page's own name.
func trail(section, leaf string) []ui.Item {
	out := []ui.Item{ui.Crumb(ui.Home(), ui.IconHouse, ui.Label(S.Common.Brand()), ui.Href("/"))}
	name := sectionName(section)
	switch {
	case name != "" && leaf == "":
		out = append(out, ui.Crumb(ui.Text(name)))
	case name != "":
		out = append(out, ui.Crumb(ui.Href(sectionHref(section)), ui.Text(name)))
	}
	if leaf != "" {
		out = append(out, ui.Crumb(ui.Text(leaf)))
	}
	return out
}

// pills are the strip of sections, the current one lit on its listing.
func pills(section, leaf string) []ui.Item {
	out := make([]ui.Item, len(sections))
	for i, x := range sections {
		pill := []ui.Item{ui.Href(x.href), ui.Text(sectionName(x.key))}
		if x.key == section && leaf == "" {
			pill = append(pill, ui.On())
		}
		out[i] = ui.Tablink(pill...)
	}
	return out
}

// head is a listing's standing explanation. The page's name is already the
// last crumb of the bar, so it is not said again in a heading.
func head(lede ...ui.Item) ui.Node {
	if len(lede) == 0 {
		return nil
	}
	return ui.Pagehead(ui.Lede(ui.Inline(lede...)))
}

func stat(label, value string) ui.Node { return ui.Stat(ui.Label(label), ui.Text(value)) }

// provBadge marks a provisional rating, as a chip beside the name.
func provBadge() ui.Item {
	return ui.Flag(ui.Title(S.Common.Prov.Title()), ui.Text(S.Common.Prov.Badge()))
}

// provMark is the chip for a provisional rating, or nil.
func provMark(prov bool) ui.Item {
	if !prov {
		return nil
	}
	return provBadge()
}

// unratedFlag marks a result the model saw but did not count, or nil.
func unratedFlag(rated bool) ui.Item {
	if rated {
		return nil
	}
	return ui.Flag(ui.Title(S.Common.Unrated.Title()), ui.Text(S.Common.Unrated.Tag()))
}

// counted is whether a result counted, as team_events records it: a result
// with no record counts.
func counted(rated *int64) bool { return rated == nil || *rated != 0 }

// nameCell is a results table's name column: the name as a link, and under it
// the muted line of flags and town, dope's way. Nil marks and an empty town
// are left out, and so is the line when nothing is on it.
func nameCell(href, name string, under ...ui.Item) ui.Node {
	kids := []ui.Item{ui.PinCol(), line(link(href, name))}
	if sub := subline(under...); sub != nil {
		kids = append(kids, sub)
	}
	return ui.Cell(kids...)
}

func subline(items ...ui.Item) ui.Node {
	out := present(items)
	if len(out) == 0 {
		return nil
	}
	return ui.Subline(ui.Inline(out...))
}

// present is items without the nils.
func present(items []ui.Item) []ui.Item {
	var out []ui.Item
	for _, it := range items {
		if it != nil {
			out = append(out, it)
		}
	}
	return out
}

// town is the town on a name's line, or nothing when there is none.
func town(t string) ui.Item {
	if t == "" {
		return nil
	}
	return ui.Text(t)
}

// line puts mixed text and inline marks on one line, dropping nils.
func line(items ...ui.Item) *ui.RunNode { return ui.Line(present(items)...) }

func link(href, text string) ui.Item { return ui.Link(ui.Href(href), ui.Text(text)) }

// space is the gap before an inline mark, or nothing when there is no mark.
func space(mark ui.Item) ui.Item {
	if mark == nil {
		return nil
	}
	return ui.Text(" ")
}

// The addresses of the entity pages.
func playerHref(id int64) string { return "/players/" + itoa(id) + "/" }
func teamHref(id int64) string   { return "/teams/" + itoa(id) + "/" }
func tourHref(id int64) string   { return "/tournaments/" + itoa(id) + "/" }

// playerName and teamName are a name, or the placeholder with the id when
// the source has none.
func playerName(name *string, id int64) string {
	if str(name) == "" {
		return S.Common.PlayerFallback(itoa(id))
	}
	return *name
}

func teamName(name *string, id int64) string {
	if str(name) == "" {
		return S.Common.TeamFallback(itoa(id))
	}
	return *name
}

func orUntitled(name string) string {
	if name == "" {
		return S.Common.Untitled()
	}
	return name
}

func orDash(s string) string {
	if s == "" {
		return dash
	}
	return s
}

func tourType(t string) string {
	switch t {
	case "in-person":
		return S.Common.Format.InPerson()
	case "strict-sync":
		return S.Common.Format.StrictSync()
	case "sync":
		return S.Common.Format.Sync()
	case "async":
		return S.Common.Format.Async()
	}
	return t
}

// numCell is a right-aligned number.
func numCell(text string, props ...ui.Item) ui.Node {
	return ui.Cell(append([]ui.Item{ui.Num(), ui.Text(text)}, props...)...)
}

func textCell(text string, props ...ui.Item) ui.Node {
	return ui.Cell(append([]ui.Item{ui.Text(text)}, props...)...)
}

// emptyCell is a cell with nothing in it, which the kit's builder writes as
// an element with no children.
func emptyCell(props ...ui.Item) ui.Node { return ui.Cell(props...) }

// dashCell is a muted dash for a number there is none of.
func dashCell() ui.Node { return numCell(dash, ui.Faint()) }

func hcell(text string, props ...ui.Item) ui.Node {
	return ui.Hcell(append([]ui.Item{ui.Text(text)}, props...)...)
}

// table is a scrolling table under its header row.
func table(header []ui.Item, rows ...ui.Item) ui.Node {
	return ui.Table(append([]ui.Item{ui.Scroll(), ui.Trow(header...)}, rows...)...)
}

// numCol is a header over a column of numbers, with its explanation on hover
// when there is one.
func numCol(text string, title ...string) ui.Node {
	if len(title) > 0 {
		return hcell(text, ui.Num(), ui.Title(title[0]))
	}
	return hcell(text, ui.Num())
}

// nameCol is the header of the name column a phone pins.
func nameCol(text string) ui.Node { return hcell(text, ui.PinCol()) }

// toneOf is the tone of a whole delta: up, down, or nothing to say.
func toneOf(v float64) ui.Item {
	switch r := math.Round(v); {
	case r > 0:
		return ui.DeltaUp
	case r < 0:
		return ui.DeltaDown
	}
	return ui.Faint()
}

// deltaCell is a signed whole delta in its tone.
func deltaCell(v float64) ui.Node { return numCell(signed(v), toneOf(v)) }

// changeCell is a week's move in the table.
func changeCell(v *float64) ui.Node {
	if v == nil {
		return dashCell()
	}
	return deltaCell(*v)
}

// gainCell is what a result added to the rating, empty when it added nothing.
func gainCell(g *float64) ui.Node {
	if g != nil && math.Round(*g) > 0 {
		return numCell(signed(*g), ui.DeltaUp)
	}
	return emptyCell(ui.Num(), ui.Faint())
}

// stepCells are a rating before, after and the step, or dashes with no rating.
func stepCells(before, after *float64) []ui.Item {
	if before == nil || after == nil {
		return []ui.Item{numCell(dash), numCell(dash), numCell(dash)}
	}
	return []ui.Item{numCell(disp(*before)), numCell(disp(*after)), deltaCell(*after - *before)}
}

// stepCols head the three stepCells.
func stepCols() []ui.Item {
	return []ui.Item{numCol(S.Common.Col.Before()), numCol(S.Common.Col.After()), numCol("Δ")}
}

// tourCols and tourCells are the date, name and format a results table
// opens with.
func tourCols() []ui.Item {
	return []ui.Item{hcell(S.Common.Col.Date()), nameCol(S.Common.Col.Tournament()), hcell(S.Common.Col.Type())}
}

func tourCells(date string, tid int64, name, typ string, rated bool) []ui.Item {
	return []ui.Item{
		textCell(date, ui.NoWrap()),
		nameCell(tourHref(tid), orUntitled(name), unratedFlag(rated)),
		textCell(tourType(typ), ui.Faint()),
	}
}

// placeOfN is a place with the size of the field beside it, muted.
func placeOfN(place string, n int64) ui.Node {
	return ui.Cell(ui.Num(), ui.NoWrap(), ui.Inline(ui.Text(place+" "), ui.Muted(ui.Text(S.Common.OfN(itoa(n))))))
}

// rosterLine is a player in a lineup: the link, the provisional chip and
// the number.
func memberLine(id int64, name string, prov bool, value *float64, more ...ui.Item) ui.Item {
	badge := provMark(prov)
	items := []ui.Item{link(playerHref(id), name), space(badge), badge, ui.Text(" "), ui.Fig(ui.Text(dispOpt(value)))}
	return ui.Member(line(append(items, more...)...))
}

// section is a titled part of a page.
func section(title string, kids ...ui.Item) ui.Node {
	return ui.Section(append([]ui.Item{ui.Subhead(ui.Text(title))}, kids...)...)
}
