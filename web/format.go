package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"
)

// disp prints a published number: on the 2019 display scale and whole. A value
// that rounds to zero from below would print "-0", which reads as a number.
func disp(v float64) string {
	r := math.Round(v)
	if r == 0 {
		r = 0
	}
	return strconv.FormatFloat(r, 'f', 0, 64)
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
		return "—"
	}
	return placeOf(*v)
}

func placeOf(p float64) string {
	if math.Abs(p-math.Round(p)) < 1e-6 {
		return strconv.FormatInt(int64(math.Round(p)), 10)
	}
	return fmt.Sprintf("%.1f", p)
}

// blank is an empty cell rather than a dash: a metric the run did not produce
// is not a value the reader should try to read.
func blank(v *float64, digits int) string {
	if v == nil {
		return ""
	}
	return strconv.FormatFloat(*v, 'f', digits, 64)
}

func opt(v *float64, digits int) string {
	if v == nil {
		return "—"
	}
	return strconv.FormatFloat(*v, 'f', digits, 64)
}

func dispOpt(v *float64) string {
	if v == nil {
		return "—"
	}
	return disp(*v)
}

func signedOpt(v *float64) string {
	if v == nil {
		return "—"
	}
	return signed(*v)
}

// spaced groups a count inside running text in threes.
func spaced(n int) string {
	d := strconv.Itoa(n)
	var b strings.Builder
	for i, c := range d {
		if i > 0 && (len(d)-i)%3 == 0 {
			b.WriteRune(' ')
		}
		b.WriteRune(c)
	}
	return b.String()
}

// w2 is a weight to two places, with a decimal comma.
func w2(x float64) string {
	return strings.Replace(fmt.Sprintf("%.2f", x), ".", ",", 1)
}

// pct is a share as a whole percentage.
func pct(x float64) string { return fmt.Sprintf("%.0f", 100*x) }

// fold is how names compare when the question is "is this the same name":
// ignoring the padding people type and the case they type it in.
func fold(s string) string { return strings.ToLower(strings.TrimSpace(s)) }

func pagesOf(n int) int {
	return max(1, (n+pageSize-1)/pageSize)
}

func itoa(n int64) string { return strconv.FormatInt(n, 10) }
