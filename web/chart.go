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

// The chart's frame in viewBox units: its size and the margins the axis
// labels sit in.
const (
	chartW = 760.0
	chartH = 210.0
	padL   = 46.0
	padR   = 10.0
	padT   = 10.0
	padB   = 24.0
	// headroom is the share of the range left free above and below the line.
	headroom = 0.08
	// bandReach is how far, as a share of the line's range, the band may push
	// the frame beyond the line.
	bandReach = 0.5
	// gridLines is how many value lines the grid aims for.
	gridLines = 4
	// labelGap and labelDrop place an axis label beside or under its line.
	labelGap  = 6
	labelDrop = 4
	// decimeter rounds a coordinate in the points data to a tenth.
	decimeter = 10
)

// chart is a line inside its uncertainty band, with a hover crosshair and, when
// more than one series is given, a switch between them. The SVG carries the
// shape; each view's data-points carries the same points in viewBox
// coordinates, which is all the crosshair in site.js needs.
func chart(ss []series, ticks []tick, label string) ui.Node {
	usable := usableSeries(ss)
	if len(usable) == 0 {
		return ui.Empty(ui.Text(sitestrings.Default.Common.Chart.Empty()))
	}
	f := newXScale(usable[0].pts)
	var views strings.Builder
	var kids []ui.Item
	for i, sr := range usable {
		views.WriteString(f.view(sr, i > 0))
		if len(usable) > 1 {
			kids = append(kids, chartTab(sr.name, i == 0))
		}
	}
	svg := fmt.Sprintf(`<svg viewBox="0 0 %.0f %.0f" role="img" aria-label="%s">%s%s<g class="c-cursor" aria-hidden="true" hidden><line class="c-rule" y1="%g" y2="%.1f"/><circle class="c-dot" r="3.5"/></g></svg>`,
		chartW, chartH, html.EscapeString(label), views.String(), f.axis(ticks), padT, chartH-padB)
	return ui.Chart(append(kids, ui.Raw(svg))...)
}

// usableSeries are the series with a line to draw.
func usableSeries(ss []series) []series {
	var out []series
	for _, sr := range ss {
		if len(sr.pts) >= 2 {
			out = append(out, sr)
		}
	}
	return out
}

func chartTab(name string, on bool) ui.Node {
	tab := []ui.Item{ui.Text(name)}
	if on {
		tab = append(tab, ui.On())
	}
	return ui.Charttab(tab...)
}

// xScale puts the releases of the first series across the frame.
type xScale struct{ x0, span float64 }

func newXScale(first []pt) xScale {
	x0, x1 := first[0].x, first[len(first)-1].x
	span := 1.0
	if x1 > x0 {
		span = x1 - x0
	}
	return xScale{x0, span}
}

func (f xScale) px(x float64) float64 { return padL + (x-f.x0)/f.span*(chartW-padL-padR) }

// yScale puts a series' values up the frame, with headroom.
type yScale struct {
	lo, hi float64
	invert bool
}

func newYScale(sr series) yScale {
	lo, hi := math.MaxFloat64, -math.MaxFloat64
	bandLo, bandHi := lo, hi
	for _, p := range sr.pts {
		lo, hi = math.Min(lo, p.mid), math.Max(hi, p.mid)
		if sr.band {
			bandLo, bandHi = math.Min(bandLo, p.mid-p.sigma), math.Max(bandHi, p.mid+p.sigma)
		}
	}
	if !(hi > lo) {
		hi = lo + 1
	}
	// The band may widen the frame by at most bandReach of the line's own
	// range, so a few uncertain early points cannot flatten the line.
	reach := (hi - lo) * bandReach
	lo, hi = math.Min(lo, math.Max(bandLo, lo-reach)), math.Max(hi, math.Min(bandHi, hi+reach))
	pad := (hi - lo) * headroom
	return yScale{lo - pad, hi + pad, sr.invert}
}

// clamp keeps a value inside the frame.
func (s yScale) clamp(y float64) float64 { return math.Max(s.lo, math.Min(s.hi, y)) }

func (s yScale) py(y float64) float64 {
	f := (s.hi - y) / (s.hi - s.lo)
	if s.invert {
		f = (y - s.lo) / (s.hi - s.lo)
	}
	return padT + f*(chartH-padT-padB)
}

// view is one series: its grid, its band and line, and its points for the
// crosshair. Every view after the first starts hidden.
func (f xScale) view(sr series, hidden bool) string {
	ys := newYScale(sr)
	var band, line strings.Builder
	for _, run := range runs(sr.pts) {
		if sr.band && len(run) > 1 {
			fmt.Fprintf(&band, `<polygon points="%s" class="c-band"/>`, f.bandPoints(run, ys))
		}
		fmt.Fprintf(&line, `<polyline points="%s" class="c-line"/>`, f.linePoints(run, ys))
	}
	attr := ""
	if hidden {
		attr = " hidden"
	}
	return fmt.Sprintf(`<g class="c-view" data-points="%s"%s>%s%s%s</g>`,
		html.EscapeString(f.pointsData(sr.pts, ys)), attr, grid(ys), band.String(), line.String())
}

// grid is the value lines with their labels, at round steps; a place axis
// starts at 1.
func grid(ys yScale) string {
	var b strings.Builder
	step := niceStep(ys.hi-ys.lo, gridLines)
	for v := math.Ceil(ys.lo/step) * step; v <= ys.hi; v += step {
		if ys.invert && v < 1 {
			continue
		}
		y := ys.py(v)
		fmt.Fprintf(&b, `<line x1="%g" y1="%.1f" x2="%.1f" y2="%.1f" class="c-grid"/><text x="%.1f" y="%.1f" class="c-axis" text-anchor="end">%s</text>`,
			padL, y, chartW-padR, y, padL-labelGap, y+labelDrop, html.EscapeString(disp(v)))
	}
	return b.String()
}

// runs are the stretches of a series without a break. A team or player
// leaves the table when it stops playing and comes back later; joining those
// two releases with a straight line would draw a rating for weeks that never
// had one, so a gap of three times the usual step breaks the run.
func runs(pts []pt) [][]pt {
	const (
		usualSteps = 3
		minGap     = 4
	)
	gap := math.Max(medianStep(pts)*usualSteps, minGap)
	var out [][]pt
	start := 0
	for j := 1; j < len(pts); j++ {
		if pts[j].x-pts[j-1].x > gap {
			out = append(out, pts[start:j])
			start = j
		}
	}
	return append(out, pts[start:])
}

func medianStep(pts []pt) float64 {
	gaps := make([]float64, 0, len(pts))
	for j := 1; j < len(pts); j++ {
		gaps = append(gaps, pts[j].x-pts[j-1].x)
	}
	sort.Float64s(gaps)
	if len(gaps) == 0 {
		return 1
	}
	return gaps[len(gaps)/2]
}

// bandPoints go along the top of the band and back along the bottom.
func (f xScale) bandPoints(run []pt, ys yScale) string {
	var poly []string
	for _, p := range run {
		poly = append(poly, fmt.Sprintf("%.1f,%.1f", f.px(p.x), ys.py(ys.clamp(p.mid+p.sigma))))
	}
	for j := len(run) - 1; j >= 0; j-- {
		poly = append(poly, fmt.Sprintf("%.1f,%.1f", f.px(run[j].x), ys.py(ys.clamp(run[j].mid-run[j].sigma))))
	}
	return strings.Join(poly, " ")
}

func (f xScale) linePoints(run []pt, ys yScale) string {
	var pl []string
	for _, p := range run {
		pl = append(pl, fmt.Sprintf("%.1f,%.1f", f.px(p.x), ys.py(p.mid)))
	}
	return strings.Join(pl, " ")
}

// pointsData is the series for the crosshair: each point's position, date
// and tooltip.
func (f xScale) pointsData(pts []pt, ys yScale) string {
	data := make([][4]any, len(pts))
	for j, p := range pts {
		data[j] = [4]any{math.Round(f.px(p.x)*decimeter) / decimeter, math.Round(ys.py(p.mid)*decimeter) / decimeter, p.date, p.text}
	}
	js, _ := json.Marshal(data)
	return string(js)
}

// axis is the dated marks under the frame, the outer two flush with its edges.
func (f xScale) axis(ticks []tick) string {
	var b strings.Builder
	for i, t := range ticks {
		anchor := "middle"
		switch {
		case i == 0:
			anchor = "start"
		case i+1 == len(ticks):
			anchor = "end"
		}
		fmt.Fprintf(&b, `<text x="%.1f" y="%.1f" class="c-axis" text-anchor="%s">%s</text>`, f.px(t.x), chartH-labelGap, anchor, html.EscapeString(t.label))
	}
	return b.String()
}

// niceStep is a step from the 1 / 2 / 2.5 / 5 ladder, so axis labels are round.
func niceStep(span float64, target int) float64 {
	raw := span / float64(target)
	if !(raw > 0) || math.IsInf(raw, 0) {
		return 1
	}
	mag := math.Pow(decimal, math.Floor(math.Log10(raw)))
	return ladder(raw/mag) * mag
}

// ladder is the smallest rung at or above n, for n in (1, 10].
func ladder(n float64) float64 {
	for _, s := range []float64{1, 2, 2.5, 5} {
		if n <= s {
			return s
		}
	}
	return 10
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
