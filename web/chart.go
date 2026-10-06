package main

import (
	"encoding/json"
	"fmt"
	"html"
	"math"
	"sort"
	"strings"

	sitestrings "ratingweb/i18nstrings"
	"ratingweb/ui"
)

// pt is one point of a history: x on the release axis, the value and its
// uncertainty in the units of its own series, and what the tooltip says.
type pt struct {
	x, mid, sigma float64
	date, text    string
}

// series is one thing a chart can show. Rating and place have different units
// and run in opposite directions, so each carries its own scale and geometry.
type series struct {
	name string
	pts  []pt
	// invert puts 1 at the top: places count downwards.
	invert bool
	band   bool
}

// chart is a line inside its uncertainty band, with a hover crosshair and, when
// more than one series is given, a switch between them. The SVG carries the
// shape; each view's data-points carries the same points in viewBox
// coordinates, which is all the crosshair in site.js needs.
func chart(ss []series, ticks []tick, label string) ui.Node {
	var usable []series
	for _, sr := range ss {
		if len(sr.pts) >= 2 {
			usable = append(usable, sr)
		}
	}
	if len(usable) == 0 {
		return ui.Empty(ui.Text(sitestrings.Default.Common.Chart.Empty()))
	}
	const (
		W = 760.0
		H = 210.0
		L = 46.0
		R = 10.0
		T = 10.0
		B = 24.0
	)
	first := usable[0].pts
	x0, x1 := first[0].x, first[len(first)-1].x
	xs := 1.0
	if x1 > x0 {
		xs = x1 - x0
	}
	px := func(x float64) float64 { return L + (x-x0)/xs*(W-L-R) }

	var views strings.Builder
	var kids []ui.Item
	for i, sr := range usable {
		lo, hi := math.MaxFloat64, -math.MaxFloat64
		for _, p := range sr.pts {
			d := 0.0
			if sr.band {
				d = p.sigma
			}
			lo = math.Min(lo, p.mid-d)
			hi = math.Max(hi, p.mid+d)
		}
		if !(hi > lo) {
			hi = lo + 1
		}
		pad := (hi - lo) * 0.08
		lo, hi = lo-pad, hi+pad
		py := func(y float64) float64 {
			f := (hi - y) / (hi - lo)
			if sr.invert {
				f = (y - lo) / (hi - lo)
			}
			return T + f*(H-T-B)
		}

		var grid strings.Builder
		step := niceStep(hi-lo, 4)
		for v := math.Ceil(lo/step) * step; v <= hi; v += step {
			if sr.invert && v < 1 {
				continue
			}
			y := py(v)
			fmt.Fprintf(&grid, `<line x1="%g" y1="%.1f" x2="%.1f" y2="%.1f" class="c-grid"/><text x="%.1f" y="%.1f" class="c-axis" text-anchor="end">%s</text>`,
				L, y, W-R, y, L-6, y+4, html.EscapeString(disp(v)))
		}

		// A team or player leaves the table when it stops playing and comes back
		// later; joining those two releases with a straight line would draw a
		// rating for weeks that never had one. Break the run instead.
		gaps := make([]float64, 0, len(sr.pts))
		for j := 1; j < len(sr.pts); j++ {
			gaps = append(gaps, sr.pts[j].x-sr.pts[j-1].x)
		}
		sort.Float64s(gaps)
		med := 1.0
		if len(gaps) > 0 {
			med = gaps[len(gaps)/2]
		}
		gap := math.Max(med*3, 4)
		var runs [][]pt
		start := 0
		for j := 1; j < len(sr.pts); j++ {
			if sr.pts[j].x-sr.pts[j-1].x > gap {
				runs = append(runs, sr.pts[start:j])
				start = j
			}
		}
		runs = append(runs, sr.pts[start:])

		var band, line strings.Builder
		for _, run := range runs {
			if sr.band && len(run) > 1 {
				var poly []string
				for _, p := range run {
					poly = append(poly, fmt.Sprintf("%.1f,%.1f", px(p.x), py(p.mid+p.sigma)))
				}
				for j := len(run) - 1; j >= 0; j-- {
					poly = append(poly, fmt.Sprintf("%.1f,%.1f", px(run[j].x), py(run[j].mid-run[j].sigma)))
				}
				fmt.Fprintf(&band, `<polygon points="%s" class="c-band"/>`, strings.Join(poly, " "))
			}
			var pl []string
			for _, p := range run {
				pl = append(pl, fmt.Sprintf("%.1f,%.1f", px(p.x), py(p.mid)))
			}
			fmt.Fprintf(&line, `<polyline points="%s" class="c-line"/>`, strings.Join(pl, " "))
		}

		data := make([][4]any, len(sr.pts))
		for j, p := range sr.pts {
			data[j] = [4]any{math.Round(px(p.x)*10) / 10, math.Round(py(p.mid)*10) / 10, p.date, p.text}
		}
		js, _ := json.Marshal(data)
		hidden := ""
		if i > 0 {
			hidden = " hidden"
		}
		fmt.Fprintf(&views, `<g class="c-view" data-points="%s"%s>%s%s%s</g>`, html.EscapeString(string(js)), hidden, grid.String(), band.String(), line.String())
		if len(usable) > 1 {
			tab := []ui.Item{ui.Text(sr.name)}
			if i == 0 {
				tab = append(tab, ui.On())
			}
			kids = append(kids, ui.Charttab(tab...))
		}
	}

	var xa strings.Builder
	for i, t := range ticks {
		anchor := "middle"
		switch {
		case i == 0:
			anchor = "start"
		case i+1 == len(ticks):
			anchor = "end"
		}
		fmt.Fprintf(&xa, `<text x="%.1f" y="%.1f" class="c-axis" text-anchor="%s">%s</text>`, px(t.x), H-6, anchor, html.EscapeString(t.label))
	}

	svg := fmt.Sprintf(`<svg viewBox="0 0 %.0f %.0f" role="img" aria-label="%s">%s%s<g class="c-cursor" aria-hidden="true" hidden><line class="c-rule" y1="%g" y2="%.1f"/><circle class="c-dot" r="3.5"/></g></svg>`,
		W, H, html.EscapeString(label), views.String(), xa.String(), T, H-B)
	kids = append(kids, ui.Raw(svg))
	return ui.Chart(kids...)
}

// niceStep is a step from the 1 / 2 / 2.5 / 5 ladder, so axis labels are round.
func niceStep(span float64, target int) float64 {
	raw := span / float64(target)
	if !(raw > 0) || math.IsInf(raw, 0) {
		return 1
	}
	mag := math.Pow(10, math.Floor(math.Log10(raw)))
	n := raw / mag
	s := 10.0
	switch {
	case n <= 1:
		s = 1
	case n <= 2:
		s = 2
	case n <= 2.5:
		s = 2.5
	case n <= 5:
		s = 5
	}
	return s * mag
}

// thin keeps at most max points of a series, always keeping the last one.
func thin(v []pt, max int) []pt {
	if len(v) <= max {
		return v
	}
	step := (len(v) + max - 1) / max
	var out []pt
	for i := 0; i < len(v); i += step {
		out = append(out, v[i])
	}
	return append(out, v[len(v)-1])
}
