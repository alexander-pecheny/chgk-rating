// Package i18nstrings is the rating site's Catalog (dopesuite's root
// docs/adr/0006): every string the site shows, one TOML file per Surface under
// ru/, generated into the typed Strings below. Edit the TOML, then run
// `just strings` in the repo root.
//
//go:generate go -C $HOME/dopesuite/scripts/i18nstringsgen run . -dir $PWD
package i18nstrings

// Default is what the site renders in. The site is Russian only.
var Default = RU
