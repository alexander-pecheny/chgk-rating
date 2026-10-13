package main

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"html"
	"math"
	"strings"
)

// metricsJSON is /method/'s forecast comparison, written by `just metrics`:
// every system's six measures on all tournaments and on rated ones alone,
// weighted and not, with each one's paired differences against every other.
//
//go:embed metrics.json
var metricsJSON []byte

// metricsMark is the line of method.md the block replaces.
const metricsMark = "<p>{{metrics}}</p>"

var metricNames = []string{"NDCG", "@3", "@10", "r", "pair", "pair@10%"}

// metricHeads are the table's column heads: pair@10% breaks to keep its column as narrow as the rest.
var metricHeads = []string{"NDCG", "@3", "@10", "r", "pair", "pair<br>@10%"}

type metricDiff struct {
	Diff []float64 `json:"diff"`
	SE   []float64 `json:"se"`
}

type metricRow struct {
	Label string                `json:"label"`
	Value []float64             `json:"value"`
	Vs    map[string]metricDiff `json:"vs"`
}

// against is the row's difference from base on measure j and its SE; nought against itself.
func (r metricRow) against(base string, j int) (float64, float64) {
	if d, ok := r.Vs[base]; ok {
		return d.Diff[j], d.SE[j]
	}
	return 0, 0
}

type metricTable struct {
	Set      string      `json:"set"`
	Weighted bool        `json:"weighted"`
	Rows     []metricRow `json:"rows"`
}

// The chart's geometry in viewBox units: a name column, then the axis.
const (
	mW, mRow, mTop, mBottom, mNames, mRight = 360.0, 26.0, 8.0, 22.0, 104.0, 12.0
	// z95 turns a standard error into the half-width of a 95% interval.
	z95 = 1.96
)

// metricsBlock is the comparison with its four switches: weighting, the
// tournaments counted, and the measure and the base of the chart. Every combination is
// rendered here and styles.css shows the chosen one, so it needs no script.
func metricsBlock() (string, error) {
	var tables []metricTable
	if err := json.Unmarshal(metricsJSON, &tables); err != nil {
		return "", err
	}
	var b strings.Builder
	b.WriteString(`<div class="metrics u-col u-gap-md"><div class="metrics-controls u-row u-wrap u-align-center u-gap-lg">`)
	fmt.Fprintf(&b, `<label class="checkbox metrics-weighted"><input type="checkbox" checked> %s</label>`, html.EscapeString(S.Method.Metrics.Weighted()))
	fmt.Fprintf(&b, `<label class="field metrics-set"><span>%s</span><select class="input"><option value="rated">%s</option><option value="all">%s</option></select></label>`,
		html.EscapeString(S.Method.Metrics.Tournaments()), html.EscapeString(S.Method.Metrics.Rated()), html.EscapeString(S.Method.Metrics.All()))
	fmt.Fprintf(&b, `<label class="field metrics-metric"><span>%s</span><select class="input">`, html.EscapeString(S.Method.Metrics.Metric()))
	for i, n := range metricNames {
		fmt.Fprintf(&b, `<option value="%d">%s</option>`, i, html.EscapeString(n))
	}
	b.WriteString(`</select></label>`)
	if len(tables) == 0 {
		return "", fmt.Errorf("metrics.json has no tables")
	}
	fmt.Fprintf(&b, `<label class="field metrics-base"><span>%s</span><select class="input">`, html.EscapeString(S.Method.Metrics.Base()))
	for k, r := range tables[0].Rows {
		fmt.Fprintf(&b, `<option value="%d">%s</option>`, k, html.EscapeString(r.Label))
	}
	b.WriteString(`</select></label></div>`)
	for _, t := range tables {
		w := "u"
		if t.Weighted {
			w = "w"
		}
		var caption strings.Builder
		for k, r := range t.Rows {
			fmt.Fprintf(&caption, `<span class="mb mb-%d">%s</span>`, k, html.EscapeString(S.Method.Metrics.Caption(r.Label)))
		}
		fmt.Fprintf(&b, `<div class="metrics-view metrics-%s-%s u-gap-lg">%s<figure class="metrics-chart">%s<figcaption class="hint">%s</figcaption></figure></div>`,
			t.Set, w, metricsTableHTML(t), metricsChart(t), caption.String())
	}
	b.WriteString(`</div>`)
	return b.String(), nil
}

// metricsTableHTML is the table as goldmark would set it, the best of each column bold.
func metricsTableHTML(t metricTable) string {
	best := make([]float64, len(metricNames))
	for j := range best {
		for _, r := range t.Rows {
			best[j] = math.Max(best[j], round3(r.Value[j]))
		}
	}
	var b strings.Builder
	fmt.Fprintf(&b, `<table class="metrics-table"><thead><tr><th style="text-align:left">%s</th>`, html.EscapeString(S.Method.Metrics.System()))
	for _, n := range metricHeads {
		fmt.Fprintf(&b, `<th style="text-align:right">%s</th>`, n)
	}
	b.WriteString(`</tr></thead><tbody>`)
	for _, r := range t.Rows {
		fmt.Fprintf(&b, `<tr><td>%s</td>`, html.EscapeString(r.Label))
		for j, v := range r.Value {
			cell := strings.TrimPrefix(fmt.Sprintf("%.3f", v), "0")
			if round3(v) == best[j] {
				cell = "<strong>" + cell + "</strong>"
			}
			fmt.Fprintf(&b, `<td style="text-align:right">%s</td>`, cell)
		}
		b.WriteString(`</tr>`)
	}
	b.WriteString(`</tbody></table>`)
	return b.String()
}

func round3(x float64) float64 { return math.Round(x*1000) / 1000 }

// metricsChart is each system's difference from a base, with its 95% interval:
// a group per base holding a group per measure. The names stay put while the
// axis changes.
func metricsChart(t metricTable) string {
	h := mTop + mRow*float64(len(t.Rows)) + mBottom
	var b strings.Builder
	fmt.Fprintf(&b, `<svg viewBox="0 0 %.0f %.0f" role="img">`, mW, h)
	for i, r := range t.Rows {
		fmt.Fprintf(&b, `<text x="0" y="%.1f" class="m-name">%s</text>`, mTop+mRow*(float64(i)+0.5)+4, html.EscapeString(r.Label))
	}
	for k, base := range t.Rows {
		fmt.Fprintf(&b, `<g class="mb mb-%d">`, k)
		for j := range metricNames {
			metricsAxis(&b, t, base.Label, j, h)
		}
		b.WriteString(`</g>`)
	}
	b.WriteString(`</svg>`)
	return b.String()
}

// metricsAxis is one measure against one base: its grid, then a dot and an interval per system.
func metricsAxis(b *strings.Builder, t metricTable, base string, j int, h float64) {
	lo, hi := 0.0, 0.0
	for _, r := range t.Rows {
		d, se := r.against(base, j)
		lo, hi = math.Min(lo, d-z95*se), math.Max(hi, d+z95*se)
	}
	pad := (hi - lo) * 0.06
	lo, hi = lo-pad, hi+pad
	x := func(v float64) float64 { return mNames + (v-lo)/(hi-lo)*(mW-mNames-mRight) }
	fmt.Fprintf(b, `<g class="m-view m-%d">`, j)
	step := niceStep(hi-lo, 4)
	for v := math.Ceil(lo/step) * step; v <= hi; v += step {
		cls := "c-grid"
		if math.Abs(v) < step/2 {
			v, cls = 0, "m-zero"
		}
		fmt.Fprintf(b, `<line x1="%.1f" x2="%.1f" y1="%.1f" y2="%.1f" class="%s"/><text x="%.1f" y="%.1f" class="c-axis" text-anchor="middle">%s</text>`,
			x(v), x(v), mTop, h-mBottom, cls, x(v), h-6, axisLabel(v, step))
	}
	for i, r := range t.Rows {
		y := mTop + mRow*(float64(i)+0.5)
		d, se := r.against(base, j)
		e := z95 * se
		if e > 0 {
			fmt.Fprintf(b, `<line x1="%.1f" x2="%.1f" y1="%.1f" y2="%.1f" class="m-ci"/>`, x(d-e), x(d+e), y, y)
		}
		fmt.Fprintf(b, `<circle cx="%.1f" cy="%.1f" r="3.5" class="m-dot"/>`, x(d), y)
	}
	b.WriteString(`</g>`)
}

// axisLabel prints an axis value with its sign and as many decimals as the step needs.
func axisLabel(v, step float64) string {
	if v == 0 {
		return "0"
	}
	digits := 0
	for scaled := step; math.Abs(scaled-math.Round(scaled)) > 1e-9 && digits < 6; scaled *= 10 {
		digits++
	}
	return strings.Replace(fmt.Sprintf("%+.*f", digits, v), "-", "−", 1)
}
