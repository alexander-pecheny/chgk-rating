module ratingweb

go 1.26.4

require (
	github.com/wyatt915/treeblood v0.1.16
	github.com/yuin/goldmark v1.8.6
	modernc.org/sqlite v1.50.0
	pecheny.me/dopecore v0.0.0
	pecheny.me/dopeuikit v0.0.0
)

require (
	github.com/dustin/go-humanize v1.0.1 // indirect
	github.com/google/uuid v1.6.0 // indirect
	github.com/mattn/go-isatty v0.0.20 // indirect
	github.com/ncruces/go-strftime v1.0.0 // indirect
	github.com/remyoudompheng/bigfft v0.0.0-20230129092748-24d4a6f8daec // indirect
	github.com/tdewolff/minify/v2 v2.24.17 // indirect
	github.com/tdewolff/parse/v2 v2.8.16 // indirect
	golang.org/x/crypto v0.52.0 // indirect
	golang.org/x/sys v0.47.0 // indirect
	modernc.org/libc v1.72.0 // indirect
	modernc.org/mathutil v1.7.1 // indirect
	modernc.org/memory v1.11.0 // indirect
)

// The kit and the platform layer are read from the dopesuite checkout at
// /home/pecheny/dopesuite, which also holds the kit's built menu.js.
replace pecheny.me/dopeuikit => /home/pecheny/dopesuite/dopeuikit

replace pecheny.me/dopecore => /home/pecheny/dopesuite/dopecore
