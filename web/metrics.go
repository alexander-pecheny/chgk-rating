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
// weighted and not, with paired differences against the published forecast.
//
//go:embed metrics.json
var metricsJSON []byte

// metricsMark is the line of method.md the block replaces.
const metricsMark = "<p>{{metrics}}</p>"

var metricNames = []string{"NDCG", "@3", "@10", "r", "pair", "pair@10%"}

type metricRow struct {
	Label string    `json:"label"`
	Value []float64 `json:"value"`
	Diff  []float64 `json:"diff"`
	SE    []float64 `json:"se"`
}

type metricTable struct {
	Set       string      `json:"set"`
	Weighted  bool        `json:"weighted"`
	Reference string      `json:"reference"`
	Rows      []metricRow `json:"rows"`
}

// The chart's geometry in viewBox units: a name column, then the axis.
const (
	mW, mRow, mTop, mBottom, mNames, mRight = 360.0, 26.0, 8.0, 22.0, 104.0, 12.0
	// z95 turns a standard error into the half-width of a 95% interval.
	z95 = 1.96
)

// metricsBlock is the comparison with its three switches: weighting, the
// tournaments counted and the measure on the chart. Every combination is
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
	b.WriteString(`</select></label></div>`)
	for _, t := range tables {
		w := "u"
		if t.Weighted {
			w = "w"
		}
		fmt.Fprintf(&b, `<div class="metrics-view metrics-%s-%s u-gap-lg">%s<figure class="metrics-chart">%s<figcaption class="hint">%s</figcaption></figure></div>`,
			t.Set, w, metricsTableHTML(t), metricsChart(t), html.EscapeString(S.Method.Metrics.Caption(t.Reference)))
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
	fmt.Fprintf(&b, `<table><thead><tr><th>%s</th>`, html.EscapeString(S.Method.Metrics.System()))
	for _, n := range metricNames {
		fmt.Fprintf(&b, `<th style="text-align:right">%s</th>`, html.EscapeString(n))
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

// metricsChart is each system's difference from the reference, with its 95%
// interval, one group per measure; the names stay put while the axis changes.
func metricsChart(t metricTable) string {
	h := mTop + mRow*float64(len(t.Rows)) + mBottom
	var b strings.Builder
	fmt.Fprintf(&b, `<svg viewBox="0 0 %.0f %.0f" role="img">`, mW, h)
	for i, r := range t.Rows {
		fmt.Fprintf(&b, `<text x="0" y="%.1f" class="m-name">%s</text>`, mTop+mRow*(float64(i)+0.5)+4, html.EscapeString(r.Label))
	}
	for j := range metricNames {
		lo, hi := 0.0, 0.0
		for _, r := range t.Rows {
			lo, hi = math.Min(lo, r.Diff[j]-z95*r.SE[j]), math.Max(hi, r.Diff[j]+z95*r.SE[j])
		}
		pad := (hi - lo) * 0.06
		lo, hi = lo-pad, hi+pad
		x := func(v float64) float64 { return mNames + (v-lo)/(hi-lo)*(mW-mNames-mRight) }
		fmt.Fprintf(&b, `<g class="m-view m-%d">`, j)
		step := niceStep(hi-lo, 4)
		for v := math.Ceil(lo/step) * step; v <= hi; v += step {
			cls := "c-grid"
			if math.Abs(v) < step/2 {
				v, cls = 0, "m-zero"
			}
			fmt.Fprintf(&b, `<line x1="%.1f" x2="%.1f" y1="%.1f" y2="%.1f" class="%s"/><text x="%.1f" y="%.1f" class="c-axis" text-anchor="middle">%s</text>`,
				x(v), x(v), mTop, h-mBottom, cls, x(v), h-6, axisLabel(v, step))
		}
		for i, r := range t.Rows {
			y := mTop + mRow*(float64(i)+0.5)
			d, e := r.Diff[j], z95*r.SE[j]
			if e > 0 {
				fmt.Fprintf(&b, `<line x1="%.1f" x2="%.1f" y1="%.1f" y2="%.1f" class="m-ci"/>`, x(d-e), x(d+e), y, y)
			}
			fmt.Fprintf(&b, `<circle cx="%.1f" cy="%.1f" r="3.5" class="m-dot"/>`, x(d), y)
		}
		b.WriteString(`</g>`)
	}
	b.WriteString(`</svg>`)
	return b.String()
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
