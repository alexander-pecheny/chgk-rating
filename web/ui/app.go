// Package ui is the rating site's thin overlay over DopeUIKit's kit
// (pecheny.me/dopeuikit/kit): the site header and footer, the page head and
// stat block, the pager, the history chart, the question strip, the lineup
// lists and the switches above the tables. It also widens the kit's own table
// cells with the roles a ranking table needs (a number column, a delta tone, a
// provisional row). tags_gen.go re-exports every kit constructor beside these,
// so the server imports this one package.
package ui

//go:generate go run pecheny.me/dopeuikit/cmd/uigen -core $HOME/dopesuite/dopeuikit/kit/vocab.json -overlay vocab.json -pkg ui -base pecheny.me/dopeuikit/kit -out tags_gen.go

import (
	_ "embed"

	base "pecheny.me/dopeuikit/kit"

	sitestrings "ratingweb/i18nstrings"
)

//go:embed vocab.json
var overlayVocab []byte

var app = mustApp()

func mustApp() *base.App {
	a, err := base.NewApp(base.Options{
		Strings:      sitestrings.Default,
		VocabOverlay: overlayVocab,
		ExtendProps: map[string][]base.PropSpec{
			"table": {{Name: "breakdown", Kind: "bare"}},
			"trow": {
				{Name: "prov", Kind: "bare"},
				{Name: "split", Kind: "bare"},
				{Name: "counts", Kind: "bare"},
				{Name: "older", Kind: "bare"},
				{Name: "foot", Kind: "bare"},
			},
			"hcell": cellProps,
			"cell":  cellProps,
		},
		Expand: expanders,
		Inline: inliners,
		Mounts: map[string]base.MountSpec{},
		Chrome: base.CoreChrome().With(base.Chrome{
			// dope's game page: the bar over a frame as wide as the screen,
			// which scrolls on the grey with the table head stuck to its top.
			DefaultKind: "site",
			PageKinds: map[string]base.PageKind{
				"site": {Body: []string{"host"}, Main: []string{"match-main"}, Frame: []string{"sheet-frame", "fight-frame"}},
			},
		}),
	})
	if err != nil {
		panic(err)
	}
	return a
}

// cellProps are the roles a ranking table gives its cells: a right-aligned
// number, the figure that matters in a row, a muted aside, a column that never
// wraps, the name column a phone pins while the rest scrolls, and the tone of
// a delta or a place.
var cellProps = []base.PropSpec{
	{Name: "num", Kind: "bare"},
	{Name: "strong", Kind: "bare"},
	{Name: "faint", Kind: "bare"},
	{Name: "nowrap", Kind: "bare"},
	{Name: "pin", Kind: "bare"},
	{Name: "tone", Enum: "cell-tone"},
	{Name: "colspan"},
}

// Render validates and prints a builder-made tree.
func Render(doc *base.Doc) ([]byte, error) { return app.Render(doc) }
