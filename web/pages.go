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

func sectionName(key string) string {
	switch key {
	case "teams":
		return S.Common.Nav.Teams()
	case "players":
		return S.Common.Nav.Players()
	case "tournaments":
		return S.Common.Nav.Tournaments()
	case "releases":
		return S.Common.Nav.Releases()
	case "method":
		return S.Common.Nav.Method()
	case "anomaly":
		return S.Anomaly.Title()
	case "search":
		return S.Search.Title()
	}
	return ""
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
	trail := []ui.Item{ui.Crumb(ui.Home(), ui.IconHouse, ui.Label(S.Common.Brand()), ui.Href("/"))}
	if name := sectionName(section); name != "" {
		if leaf == "" {
			trail = append(trail, ui.Crumb(ui.Text(name)))
		} else {
			trail = append(trail, ui.Crumb(ui.Href(sectionHref(section)), ui.Text(name)))
		}
	}
	if leaf != "" {
		trail = append(trail, ui.Crumb(ui.Text(leaf)))
	}
	pills := make([]ui.Item, len(sections))
	for i, x := range sections {
		pill := []ui.Item{ui.Href(x.href), ui.Text(sectionName(x.key))}
		if x.key == section && leaf == "" {
			pill = append(pill, ui.On())
		}
		pills[i] = ui.Tablink(pill...)
	}
	items := []ui.Item{
		ui.Title(title), ui.PageSite, ui.Classicscripts("site.js"),
		ui.Sitebar(ui.Crumbs(trail...), ui.Tablinks(pills...)),
	}
	content := append(body, ui.Sitefoot(ui.Inline(
		ui.Text(S.Common.Foot.Note()),
		ui.Link(ui.Href("/report.pdf"), ui.Text(S.Common.Foot.Report())),
	)))
	items = append(items, ui.Sitebody(content...))
	return &ui.Doc{Nodes: []ui.Node{ui.Page(items...)}}
}

// head is a listing's standing explanation. The page's name is already the
// last crumb of the bar, so it is not said again in a heading.
func head(title string, lede ...ui.Item) ui.Node {
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

// unratedFlag marks a result the model saw but did not count, or nil.
func unratedFlag(rated bool) ui.Item {
	if rated {
		return nil
	}
	return ui.Flag(ui.Title(S.Common.Unrated.Title()), ui.Text(S.Common.Unrated.Tag()))
}

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
	var out []ui.Item
	for _, it := range items {
		if it != nil {
			out = append(out, it)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return ui.Subline(ui.Inline(out...))
}

// town is the town on a name's line, or nothing when there is none.
func town(t string) ui.Item {
	if t == "" {
		return nil
	}
	return ui.Text(t)
}

// line puts mixed text and inline marks on one line, dropping nils.
func line(items ...ui.Item) *ui.RunNode {
	var out []ui.Item
	for _, it := range items {
		if it != nil {
			out = append(out, it)
		}
	}
	return ui.Line(out...)
}

func link(href, text string) ui.Item { return ui.Link(ui.Href(href), ui.Text(text)) }

func orUntitled(name string) string {
	if name == "" {
		return S.Common.Untitled()
	}
	return name
}

func orDash(s string) string {
	if s == "" {
		return "—"
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

func hcell(text string, props ...ui.Item) ui.Node {
	return ui.Hcell(append([]ui.Item{ui.Text(text)}, props...)...)
}

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

// changeCell is a week's move in the table.
func changeCell(v *float64) ui.Node {
	if v == nil {
		return numCell("—", ui.Faint())
	}
	return numCell(signed(*v), toneOf(*v))
}

// gainCell is what a result added to the rating, empty when it added nothing.
func gainCell(g *float64) ui.Node {
	if g != nil && math.Round(*g) > 0 {
		return numCell(signed(*g), ui.DeltaUp)
	}
	return emptyCell(ui.Num(), ui.Faint())
}

// stepCells are a rating before, after and the step, or dashes with no rating.
func stepCells(a, b float64, before, after *float64) []ui.Item {
	if before == nil || after == nil {
		return []ui.Item{numCell("—"), numCell("—"), numCell("—")}
	}
	x, y := a+b**before, a+b**after
	return []ui.Item{numCell(disp(x)), numCell(disp(y)), numCell(signed(y-x), toneOf(y-x))}
}

// provToggle is the provisional switch. It is off by default, so a reader with
// no script sees only settled ratings; site.js remembers the choice.
func provToggle(hidden int, firstPage int, page int) ui.Node {
	var note string
	switch {
	case hidden > 0:
		note = S.Common.Prov.Hidden(itoa(int64(hidden)))
	case firstPage > page:
		note = S.Common.Prov.FromPage(itoa(int64(firstPage)))
	}
	items := []ui.Item{ui.Text(S.Common.Prov.Toggle())}
	if note != "" {
		items = append(items, ui.Text(" "), ui.Count(ui.Text(note)))
	}
	return ui.Toggle(ui.ToggleProv, ui.Inline(items...))
}

func pager(page, pages int, first, prefix string) ui.Node {
	return ui.Pager(ui.Cur(itoa(int64(page))), ui.Total(itoa(int64(pages))), ui.First(first), ui.Prefix(prefix))
}
