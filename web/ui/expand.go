package ui

import (
	"strconv"

	"pecheny.me/dopeuikit/icons"
	base "pecheny.me/dopeuikit/kit"

	sitestrings "ratingweb/i18nstrings"
)

// The class names below are the site layer's, matched by static/styles.css; the
// ones it shares with the kit (host-top, host-brand, host-actions, checkbox,
// field, input, data-table, table-scroll, scroll-fade-xy) are core.css's.

var expanders = map[string]base.ExpandFunc{
	"sitebar": expandSitebar,
	"tablinks": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.El("nav", base.RootAttrs([]string{"match-tabs", "site-sections"}, p), c.Nodes(p.Block)...))
	},
	"tablink": expandTablink,
	"subline": expandSubline,
	"flag":    func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(inlineFlag(c, p).(*base.Element)) },
	"sitefoot": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.Leaf(c, "footer", []string{"site-foot"}, nil, p))
	},
	"pagehead": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.El("header", base.RootAttrs([]string{"page-head"}, p), c.Nodes(p.Block)...))
	},
	"lede": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.Leaf(c, "p", []string{"lede"}, nil, p))
	},
	"byline": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.Leaf(c, "p", []string{"muted"}, nil, p))
	},
	"stats": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.El("dl", base.RootAttrs([]string{"stats"}, p), c.Nodes(p.Block)...))
	},
	"stat":     expandStat,
	"pager":    expandPager,
	"chart":    expandChart,
	"charttab": expandCharttab,
	"qbars": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.El("div", base.RootAttrs([]string{"qbars"}, p), c.Nodes(p.Block)...))
	},
	"qbar":   expandQbar,
	"toggle": expandToggle,
	"count":  func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(inlineCount(c, p).(*base.Element)) },
	"link":   func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(inlineLink(c, p).(*base.Element)) },
	"sup":    func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(inlineSup(c, p).(*base.Element)) },
	"fig":    func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(inlineFig(c, p).(*base.Element)) },
	"em":     func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(inlineEm(c, p).(*base.Element)) },
	"dotin":  func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(inlineDotin(c, p).(*base.Element)) },
	"how": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.Leaf(c, "div", []string{"how"}, nil, p))
	},
	"roster": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.El("ul", base.RootAttrs([]string{"roster", "base-roster"}, p), c.Nodes(p.Block)...))
	},
	"cellroster": func(c *base.ExpandCtx, p *base.Element) []base.Node {
		return one(base.El("ul", base.RootAttrs([]string{"cell-roster"}, p), c.Nodes(p.Block)...))
	},
	"member":      func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(base.Leaf(c, "li", nil, nil, p)) },
	"rosteropen":  expandRosteropen,
	"venuefilter": expandVenuefilter,
	"explainer":   expandExplainer,
	"table":       expandTable,
	"trow":        expandTrow,
	"hcell":       func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(cell(c, "th", p)) },
	"cell":        func(c *base.ExpandCtx, p *base.Element) []base.Node { return one(cell(c, "td", p)) },
}

var inliners = map[string]base.InlineFunc{
	"link":  inlineLink,
	"count": inlineCount,
	"sup":   inlineSup,
	"flag":  inlineFlag,
	"fig":   inlineFig,
	"em":    inlineEm,
	"dotin": inlineDotin,
}

func one(e *base.Element) []base.Node { return []base.Node{e} }

// expandSitebar is the bar on every page, dope's public one: the breadcrumb
// trail from the site's home to the page, and the search box. The kit's
// menu.js mounts the ☰ menu (theme, contrast, body face) into .host-actions
// beside the search.
func expandSitebar(c *base.ExpandCtx, p *base.Element) []base.Node {
	s := sitestrings.Default
	var trail []base.Node
	for _, n := range p.Block {
		if e, ok := n.(*base.Element); ok && e.Tag == "crumbs" {
			trail = append(trail, base.ExpandCrumbs(c, e)...)
		}
	}
	glyph, _ := icons.Body("search")
	search := base.El("form", []base.Attr{base.ClassAttr("site-search"), base.At("action", "/search/"), base.At("method", "get"), base.At("role", "search")},
		base.Raw(`<svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">`+glyph+`</svg>`),
		base.El("input", []base.Attr{
			base.ClassAttr("input"),
			base.At("type", "search"), base.At("name", "q"),
			base.At("placeholder", s.Common.Search.Placeholder()),
			base.At("aria-label", s.Common.Search.Label()),
			base.At("autocomplete", "off"),
		}),
	)
	return one(base.El("header", base.RootAttrs([]string{"public-top"}, p),
		base.El("div", []base.Attr{base.ClassAttr("site-trail")}, trail...),
		base.El("div", []base.Attr{base.ClassAttr("host-actions")}, search),
	))
}

func expandStat(c *base.ExpandCtx, p *base.Element) []base.Node {
	label, _ := base.Get(p, "label")
	return one(base.El("div", base.RootAttrs([]string{"stat"}, p),
		base.Inl("dt", nil, &base.TextNode{Value: label}),
		&base.Element{Tag: "dd", Inline: c.Items(p.Inline)},
	))
}

// expandPager is the strip of page numbers over and under a listing, set as
// pills. The listings run to dozens of pages, so it shows the two ends, the
// neighbourhood of the current page and an ellipsis for each stretch between.
// Page 1 lives at `first`, page n at `prefix` + n + "/".
func expandPager(c *base.ExpandCtx, p *base.Element) []base.Node {
	cur := atoi(p, "cur")
	total := atoi(p, "total")
	first, _ := base.Get(p, "first")
	prefix, _ := base.Get(p, "prefix")
	if total < 2 {
		return nil
	}
	var kids []base.Node
	last := 0
	for i := 1; i <= total; i++ {
		near := i <= 2 || i+2 > total || abs(i-cur) <= 2
		if !near {
			continue
		}
		if i > last+1 {
			kids = append(kids, base.Inl("span", []base.Attr{base.ClassAttr("pager-gap")}, &base.TextNode{Value: "…"}))
		}
		n := strconv.Itoa(i)
		href := prefix + n + "/"
		if i == 1 {
			href = first
		}
		switch {
		case i == cur:
			kids = append(kids, base.Inl("span", []base.Attr{base.ClassAttr("match-tab", "active"), base.At("aria-current", "page")}, &base.TextNode{Value: n}))
		default:
			kids = append(kids, base.Inl("a", []base.Attr{base.ClassAttr("match-tab"), base.At("href", href)}, &base.TextNode{Value: n}))
		}
		last = i
	}
	return one(base.El("nav", base.RootAttrs([]string{"match-tabs", "pager"}, p, base.At("aria-label", sitestrings.Default.Common.Pager.Label())), kids...))
}

// expandChart frames a history chart. Its charttab children become the switch
// between the series, the rest (the SVG the server drew) goes in as it is, and
// the tooltip site.js fills follows.
func expandChart(c *base.ExpandCtx, p *base.Element) []base.Node {
	var tabs, body []base.Node
	for _, n := range p.Block {
		if e, ok := n.(*base.Element); ok && e.Tag == "charttab" {
			tabs = append(tabs, c.Expand(e)...)
			continue
		}
		body = append(body, c.Nodes([]base.Node{n})...)
	}
	var kids []base.Node
	if len(tabs) > 0 {
		kids = append(kids, base.El("div", []base.Attr{base.ClassAttr("c-tabs"), base.At("role", "group")}, tabs...))
	}
	kids = append(kids, body...)
	kids = append(kids, base.El("div", []base.Attr{base.ClassAttr("c-tip"), base.BareAt("hidden")}))
	return one(base.El("figure", base.RootAttrs([]string{"chart", "scroll-fade-xy"}, p), kids...))
}

func expandCharttab(c *base.ExpandCtx, p *base.Element) []base.Node {
	classes := []string{"c-tab"}
	pressed := "false"
	if base.Flag(p, "on") {
		classes, pressed = append(classes, "on"), "true"
	}
	return one(base.Leaf(c, "button", classes, []base.Attr{base.At("type", "button"), base.At("aria-pressed", pressed)}, p))
}

// expandQbar is one question of the difficulty strip, its height a share of the
// strip. The height is the one inline style on the site: it is data, not design.
func expandQbar(c *base.ExpandCtx, p *base.Element) []base.Node {
	h, _ := base.Get(p, "h")
	attrs := []base.Attr{base.At("style", "height:"+h+"%")}
	return one(base.El("i", append(attrs, base.MetaAttrs(p)...)))
}

// expandToggle is a switch above a table that shows the rows it hides by
// default: provisional ratings, results older than three years, lineups. The
// CSS reads it with :has(), so it works with no script; site.js only remembers
// the choice.
func expandToggle(c *base.ExpandCtx, p *base.Element) []base.Node {
	kind, _ := base.Get(p, "kind")
	input := base.El("input", []base.Attr{base.At("type", "checkbox")})
	items := append([]base.Item{input}, c.Items(p.Inline)...)
	return one(&base.Element{Tag: "label", Attrs: base.RootAttrs([]string{"checkbox", kind + "-toggle"}, p), Inline: items})
}

func inlineCount(c *base.ExpandCtx, p *base.Element) base.Item {
	return &base.Element{Tag: "span", Attrs: base.RootAttrs([]string{"prov-count"}, p), Inline: c.Items(p.Inline)}
}

func inlineLink(c *base.ExpandCtx, p *base.Element) base.Item {
	href, _ := base.Get(p, "href")
	attrs := []base.Attr{base.At("href", href)}
	return &base.Element{Tag: "a", Attrs: append(attrs, base.MetaAttrs(p)...), Inline: c.Items(p.Inline)}
}

// inlineFlag is a fact about a row stated quietly beside its name: a
// provisional rating, a result that does not count. It is dope's зачёт chip.
func inlineFlag(c *base.ExpandCtx, p *base.Element) base.Item {
	return &base.Element{Tag: "span", Attrs: base.RootAttrs([]string{"team-flag"}, p), Inline: c.Items(p.Inline)}
}

// expandSubline is the muted line under a name in a results table: its flags,
// then its town, the way dope sets a team's зачёты and city.
func expandSubline(c *base.ExpandCtx, p *base.Element) []base.Node {
	return one(&base.Element{Tag: "span", Attrs: base.RootAttrs([]string{"u-row", "u-gap-xs", "u-align-center", "results-team-city"}, p), Inline: c.Items(p.Inline)})
}

// expandTablink is one pill of a strip of views: a section of the site, a
// page of a listing. The one the reader is on is the active pill and no link.
func expandTablink(c *base.ExpandCtx, p *base.Element) []base.Node {
	if base.Flag(p, "on") {
		return one(&base.Element{Tag: "span", Attrs: []base.Attr{base.ClassAttr("match-tab", "active"), base.At("aria-current", "page")}, Inline: c.Items(p.Inline)})
	}
	href, _ := base.Get(p, "href")
	return one(&base.Element{Tag: "a", Attrs: []base.Attr{base.ClassAttr("match-tab"), base.At("href", href)}, Inline: c.Items(p.Inline)})
}

// inlineFig is a number inside running text, in tabular figures.
func inlineFig(c *base.ExpandCtx, p *base.Element) base.Item {
	return &base.Element{Tag: "span", Attrs: []base.Attr{base.ClassAttr("num")}, Inline: c.Items(p.Inline)}
}

func inlineSup(c *base.ExpandCtx, p *base.Element) base.Item {
	return &base.Element{Tag: "sup", Inline: c.Items(p.Inline)}
}

func inlineEm(c *base.ExpandCtx, p *base.Element) base.Item {
	return &base.Element{Tag: "em", Inline: c.Items(p.Inline)}
}

// inlineDotin marks a result that is part of the rating today.
func inlineDotin(c *base.ExpandCtx, p *base.Element) base.Item {
	title := sitestrings.Default.Team.Results.CountsTitle()
	return &base.Element{Tag: "span", Attrs: []base.Attr{base.ClassAttr("dot-in"), base.At("title", title)}, Inline: []base.Item{&base.TextNode{Value: "●"}}}
}

// expandRosteropen is the small switch beside a team's name that opens its
// lineup in the same cell.
func expandRosteropen(c *base.ExpandCtx, p *base.Element) []base.Node {
	return one(&base.Element{Tag: "label", Attrs: base.RootAttrs([]string{"roster-open"}, p), Inline: []base.Item{
		base.El("input", []base.Attr{base.At("type", "checkbox")}),
		&base.Element{Tag: "span", Inline: []base.Item{&base.TextNode{Value: sitestrings.Default.Tournament.Field.RosterOpen()}}},
	}})
}

// expandVenuefilter is the venue select of a field played in several cities.
func expandVenuefilter(c *base.ExpandCtx, p *base.Element) []base.Node {
	label, _ := base.Get(p, "label")
	return one(base.El("label", base.RootAttrs([]string{"field", "venue-filter"}, p),
		base.Inl("span", nil, &base.TextNode{Value: label}),
		base.El("select", []base.Attr{base.ClassAttr("input")}, c.Nodes(p.Block)...),
	))
}

// expandExplainer is a folded explanation under a listing's head.
func expandExplainer(c *base.ExpandCtx, p *base.Element) []base.Node {
	summary, _ := base.Get(p, "label")
	kids := append([]base.Node{base.Inl("summary", nil, &base.TextNode{Value: summary})}, c.Nodes(p.Block)...)
	return one(base.El("details", base.RootAttrs([]string{"note"}, p), kids...))
}

// expandTable is dope's results table: a sticky header on the page grey, and
// every row a paper card. A row flagged `foot` goes to <tfoot>, a row of
// header cells to <thead>, the rest to <tbody>. The scroll wrapper only
// scrolls on a phone; on a wide screen the header sticks to the page.
func expandTable(c *base.ExpandCtx, p *base.Element) []base.Node {
	var head, body, foot []base.Node
	for _, ch := range p.Block {
		row, ok := ch.(*base.Element)
		if !ok || row.Tag != "trow" {
			continue
		}
		tr := c.Expand(row)
		switch {
		case base.Flag(row, "foot"):
			foot = append(foot, tr...)
		case headerRow(row):
			head = append(head, tr...)
		default:
			body = append(body, tr...)
		}
	}
	var kids []base.Node
	if len(head) > 0 {
		kids = append(kids, base.El("thead", nil, head...))
	}
	if len(body) > 0 {
		kids = append(kids, base.El("tbody", nil, body...))
	}
	if len(foot) > 0 {
		kids = append(kids, base.El("tfoot", nil, foot...))
	}
	classes := []string{"results-table", "rating-table"}
	if base.Flag(p, "breakdown") {
		classes = append(classes, "breakdown")
	}
	table := base.El("table", base.RootAttrs(classes, p), kids...)
	return one(base.El("div", []base.Attr{base.ClassAttr("results-wrapper", "rating-scroll")}, table))
}

func headerRow(row *base.Element) bool {
	saw := false
	for _, ch := range row.Block {
		if e, ok := ch.(*base.Element); ok {
			if e.Tag != "hcell" {
				return false
			}
			saw = true
		}
	}
	return saw
}

func expandTrow(c *base.ExpandCtx, p *base.Element) []base.Node {
	var classes []string
	if !headerRow(p) {
		classes = []string{"results-row", "results-group-first", "results-group-last"}
	}
	for _, f := range [][2]string{{"split", "row-split"}, {"prov", "prov-row"}, {"counts", "counts"}, {"older", "older-row"}} {
		if base.Flag(p, f[0]) {
			classes = append(classes, f[1])
		}
	}
	return one(base.El("tr", base.RootAttrs(classes, p), c.Nodes(p.Block)...))
}

var tones = map[string]string{"up": "amount-positive", "down": "amount-negative", "better": "amount-positive", "worse": "amount-negative"}

// cell maps a cell's roles onto the results table's classes: a number is
// centred, the name column is the team column dope pins, and a column of
// words keeps to the left like the name does.
func cell(c *base.ExpandCtx, tag string, p *base.Element) *base.Element {
	var classes []string
	switch {
	case base.Flag(p, "pin") && tag == "th":
		classes = append(classes, "results-team-head", "pin-name")
	case base.Flag(p, "pin"):
		classes = append(classes, "results-team")
	case base.Flag(p, "num"):
		classes = append(classes, "results-num")
	default:
		classes = append(classes, "rating-text")
	}
	for _, f := range [][2]string{{"faint", "muted"}, {"strong", "rating-strong"}, {"nowrap", "nowrap"}} {
		if base.Flag(p, f[0]) {
			classes = append(classes, f[1])
		}
	}
	if t, ok := base.Get(p, "tone"); ok {
		classes = append(classes, tones[t])
	}
	var extra []base.Attr
	if v, ok := base.Get(p, "colspan"); ok {
		extra = append(extra, base.At("colspan", v))
	}
	return base.Leaf(c, tag, classes, extra, p)
}

func atoi(p *base.Element, name string) int {
	v, _ := base.Get(p, name)
	n, _ := strconv.Atoi(v)
	return n
}

func abs(x int) int {
	if x < 0 {
		return -x
	}
	return x
}
