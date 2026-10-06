package ui

import base "pecheny.me/dopeuikit/kit"

// The props app.go adds to the kit's own table primitives. uigen generates
// helpers only for the props vocab.json declares, so these are written by hand.

// Cell roles, on hcell and cell.
func Num() base.Attr     { return bare("num") }
func Bold() base.Attr    { return bare("strong") }
func Faint() base.Attr   { return bare("faint") }
func NoWrap() base.Attr  { return bare("nowrap") }
func TownCol() base.Attr { return bare("town") }

// PinCol marks the name column a phone keeps in place while the rest scrolls.
func PinCol() base.Attr { return bare("pin") }

func Colspan(n string) base.Attr { return base.Attr{Name: "colspan", Value: n} }

// The tone of a delta or a place, on a cell.
var (
	DeltaUp     = base.Attr{Name: "tone", Value: "up"}
	DeltaDown   = base.Attr{Name: "tone", Value: "down"}
	PlaceBetter = base.Attr{Name: "tone", Value: "better"}
	PlaceWorse  = base.Attr{Name: "tone", Value: "worse"}
)

// Row kinds, on trow: the band before the provisional ratings, a provisional
// row, a result that counts today, a result older than three years, a footer.
func SplitRow() base.Attr  { return bare("split") }
func ProvRow() base.Attr   { return bare("prov") }
func CountsRow() base.Attr { return bare("counts") }
func OlderRow() base.Attr  { return bare("older") }
func FootRow() base.Attr   { return bare("foot") }

// Breakdown is the team page's table of the results its rating is the sum of.
func Breakdown() base.Attr { return bare("breakdown") }

func bare(name string) base.Attr { return base.Attr{Name: name, Bare: true} }
