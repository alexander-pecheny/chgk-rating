package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"
)

// dash stands in for a number there is none of.
const dash = "—"

// percent turns a share into percentage points.
const percent = 100

// decimal is the base counts are written in.
const decimal = 10

// float64Bits is the precision strconv is asked to format at.
const float64Bits = 64

// disp prints a published number: on the 2019 display scale and whole. A value
// that rounds to zero from below would print "-0", which reads as a number.
func disp(v float64) string {
	r := math.Round(v)
	if r == 0 {
		r = 0
	}
	return strconv.FormatFloat(r, 'f', 0, float64Bits)
}

// signed is a whole delta with its sign, a typographic minus below zero.
func signed(v float64) string {
	r := math.Round(v)
	switch {
	case r > 0:
		return "+" + disp(r)
	case r < 0:
		return "−" + disp(-r)
	}
	return "0"
}

// signed1 is a signed percentage-point figure, to one decimal.
func signed1(v float64) string {
	if v < 0 {
		return fmt.Sprintf("−%.1f", -v)
	}
	return fmt.Sprintf("+%.1f", v)
}

// place is a place in a field: whole, or to one decimal when it is shared.
func place(v *float64) string {
	if v == nil {
		return dash
	}
	return placeOf(*v)
}

func placeOf(p float64) string {
	if math.Abs(p-math.Round(p)) < 1e-6 {
		return strconv.FormatInt(int64(math.Round(p)), decimal)
	}
	return fmt.Sprintf("%.1f", p)
}

// blank is an empty cell rather than a dash: a metric the run did not produce
// is not a value the reader should try to read.
func blank(v *float64, digits int) string {
	if v == nil {
		return ""
	}
	return strconv.FormatFloat(*v, 'f', digits, float64Bits)
}

func opt(v *float64, digits int) string {
	if v == nil {
		return dash
	}
	return strconv.FormatFloat(*v, 'f', digits, float64Bits)
}

func dispOpt(v *float64) string {
	if v == nil {
		return dash
	}
	return disp(*v)
}

func signedOpt(v *float64) string {
	if v == nil {
		return dash
	}
	return signed(*v)
}

// intOpt is a count, or a dash when there is none.
func intOpt(v *int64) string {
	if v == nil {
		return dash
	}
	return itoa(*v)
}

// spaced groups a count inside running text in threes.
func spaced(n int) string {
	const group = 3
	d := strconv.Itoa(n)
	var b strings.Builder
	for i, c := range d {
		if i > 0 && (len(d)-i)%group == 0 {
			b.WriteRune('\u2009') // a thin space
		}
		b.WriteRune(c)
	}
	return b.String()
}

// w2 is a weight to two places.
func w2(x float64) string {
	return fmt.Sprintf("%.2f", x)
}

// pct is a share as a whole percentage.
func pct(x float64) string {
	return fmt.Sprintf("%.0f", percent*x)
}

// fold is how names compare when the question is "is this the same name":
// ignoring the padding people type and the case they type it in.
func fold(s string) string { return strings.ToLower(strings.TrimSpace(s)) }

func itoa(n int64) string { return strconv.FormatInt(n, decimal) }

func abs(x float64) float64 {
	if x < 0 {
		return -x
	}
	return x
}

func prefix(s string, n int) string {
	if len(s) < n {
		return s
	}
	return s[:n]
}

// modelDigits is how many decimals the model's own numbers get, as on /method/.
const modelDigits = 3

// model is one of the model's numbers, θ, σ or v, with a typographic minus.
// Every number on the site takes a decimal point, as the tables of places
// and weights always have.
func model(v float64) string {
	return strings.Replace(strconv.FormatFloat(v, 'f', modelDigits, float64Bits), "-", "−", 1)
}

// plainNum is a number with as many decimals as it needs.
func plainNum(v float64) string {
	return strconv.FormatFloat(v, 'f', -1, float64Bits)
}
