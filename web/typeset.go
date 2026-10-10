package main

import (
	"embed"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
)

// The post's formulas are drawn by Typst, as SVG, when the server starts:
// browsers' own MathML layout sets sums, accents and scripts far worse than
// a typesetter does. Typst reads the TeX through the mitex package, which it
// fetches into its cache on the first run.

// typstBin is the typst executable; -typst sets it.
var typstBin = "typst"

// The fonts are the whole files: Typst reads them here, and a reader gets
// only the outlines of the glyphs a formula uses.
//
//go:embed math/*.ttf
var mathFonts embed.FS

// The size Typst sets the formulas in, which the page's em stands for.
const texPt = 10

// typeset sets every formula, giving each part of it its SVG, and returns
// the one hidden SVG that holds the glyphs all of them draw.
func typeset(all []*tex) (string, error) {
	var parts []string
	var display []bool
	for _, t := range all {
		t.pieces = [][]string{{t.src}}
		if t.display {
			t.pieces = splitDisplay(t.src)
		}
		for _, piece := range t.pieces {
			for _, p := range piece {
				parts = append(parts, p)
				display = append(display, t.display)
			}
		}
	}
	dir, err := os.MkdirTemp("", "ratingweb-tex-")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(dir)
	if err := writeFonts(dir); err != nil {
		return "", err
	}
	doc := filepath.Join(dir, "formulas.typ")
	if err := os.WriteFile(doc, []byte(typstDoc(parts, display)), 0o644); err != nil {
		return "", err
	}
	fonts := []string{"--root", dir, "--ignore-system-fonts", "--font-path", filepath.Join(dir, "fonts")}
	if err := run(append([]string{"compile"}, append(fonts, doc, filepath.Join(dir, "{p}.svg"))...)...); err != nil {
		return "", err
	}
	out, err := exec.Command(typstBin, append([]string{"eval", "query(<base>).map(it => it.location().position().y.pt())", "--in", doc}, fonts...)...).Output()
	if err != nil {
		return "", typstErr(err)
	}
	var baselines []float64
	if err := json.Unmarshal(out, &baselines); err != nil || len(baselines) != len(parts) {
		return "", fmt.Errorf("typst gave %d baselines for %d formulas: %v", len(baselines), len(parts), err)
	}
	glyphs := newGlyphs()
	k := 0
	for _, t := range all {
		t.svgs = make([][]string, len(t.pieces))
		for i, piece := range t.pieces {
			for range piece {
				raw, err := os.ReadFile(filepath.Join(dir, strconv.Itoa(k+1)+".svg"))
				if err != nil {
					return "", err
				}
				t.svgs[i] = append(t.svgs[i], glyphs.take(string(raw), baselines[k]))
				k++
			}
		}
	}
	return glyphs.sprite(), nil
}

func writeFonts(dir string) error {
	if err := os.Mkdir(filepath.Join(dir, "fonts"), 0o755); err != nil {
		return err
	}
	names, _ := mathFonts.ReadDir("math")
	for _, f := range names {
		b, err := mathFonts.ReadFile("math/" + f.Name())
		if err == nil {
			err = os.WriteFile(filepath.Join(dir, "fonts", f.Name()), b, 0o644)
		}
		if err != nil {
			return err
		}
	}
	return nil
}

// typstDoc puts each piece on a page of its own, as tight as its ink, with a
// mark at its baseline that typeset reads back to align it with the text.
func typstDoc(pieces []string, display []bool) string {
	var b strings.Builder
	b.WriteString(`#import "@preview/mitex:0.2.6": mitex-convert, mitex-scope
#set page(width: auto, height: auto, margin: 0pt, fill: none)
#set text(font: "Noto Sans", size: ` + strconv.Itoa(texPt) + `pt, top-edge: "bounds", bottom-edge: "bounds", lang: "ru")
#show math.equation: set text(font: "Noto Sans Math")
#let tex(src, display) = {
  let f = eval("$" + mitex-convert(src) + "$", scope: mitex-scope)
  math.equation(block: false, if display { math.display(f) } else { f })
}
`)
	for i, p := range pieces {
		if i > 0 {
			b.WriteString("#pagebreak()\n")
		}
		fmt.Fprintf(&b, "#tex(\"%s\", %t)#box(width: 0pt, height: 0pt)[#metadata(none)<base>]\n", typstString(p), display[i])
	}
	return b.String()
}

func typstString(s string) string {
	return strings.NewReplacer(`\`, `\\`, `"`, `\"`, "\n", " ").Replace(primes(s))
}

var primeRun = regexp.MustCompile(`'+`)

// primes writes t' as t^{\prime}: inside a superscript Typst sets a bare
// prime full size on the baseline, but \prime as a script of its own.
func primes(tex string) string {
	return primeRun.ReplaceAllStringFunc(tex, func(run string) string {
		return `^{` + strings.Repeat(`\prime`, len(run)) + `}`
	})
}

func run(args ...string) error {
	_, err := exec.Command(typstBin, args...).Output()
	return typstErr(err)
}

func typstErr(err error) error {
	var x *exec.ExitError
	if errors.As(err, &x) {
		return fmt.Errorf("typst: %s", x.Stderr)
	}
	if err != nil {
		return fmt.Errorf("typst (set -typst to the executable): %w", err)
	}
	return nil
}

// splitDisplay cuts a display formula into pieces at each \qquad, and a
// piece into parts at each \allowbreak, outside braces and environments.
// Pieces stand apart and parts touch, but either wraps onto a line of its own
// on a narrow screen. A part after a break starts with an empty \mathord,
// so that the operator it opens keeps its space on the left.
func splitDisplay(src string) [][]string {
	var out [][]string
	for _, piece := range splitAt(src, `\qquad`) {
		parts := splitAt(piece, `\allowbreak`)
		for i := 1; i < len(parts); i++ {
			parts[i] = `\mathord{} ` + parts[i]
		}
		out = append(out, parts)
	}
	return out
}

func splitAt(src, cmd string) []string {
	var pieces []string
	depth, start := 0, 0
	for i := 0; i < len(src); i++ {
		switch {
		case src[i] == '\\' && strings.HasPrefix(src[i:], `\begin{`):
			depth++
		case src[i] == '\\' && strings.HasPrefix(src[i:], `\end{`):
			depth--
		case src[i] == '\\' && depth == 0 && strings.HasPrefix(src[i:], cmd) && !isLetter(src, i+len(cmd)):
			pieces = append(pieces, strings.TrimSpace(src[start:i]))
			start = i + len(cmd)
		case src[i] == '\\':
			i++ // an escaped brace does not nest
		case src[i] == '{':
			depth++
		case src[i] == '}':
			depth--
		}
	}
	pieces = append(pieces, strings.TrimSpace(src[start:]))
	var out []string
	for _, p := range pieces {
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

func isLetter(s string, i int) bool {
	return i < len(s) && (s[i] >= 'a' && s[i] <= 'z' || s[i] >= 'A' && s[i] <= 'Z')
}

// glyphs gathers the outlines Typst writes into every SVG, keyed by its
// hash of the outline, so that the page carries each glyph once.
type glyphs struct {
	seen  map[string]bool
	order []string
}

func newGlyphs() *glyphs { return &glyphs{seen: map[string]bool{}} }

var (
	svgSize   = regexp.MustCompile(`^<svg viewBox="0 0 ([0-9.]+) ([0-9.]+)" width="[0-9.]+pt" height="[0-9.]+pt"`)
	svgDefs   = regexp.MustCompile(`(?s)<defs>.*?</defs>`)
	svgSymbol = regexp.MustCompile(`(?s)<symbol id="([^"]+)".*?</symbol>`)
	// A formula is drawn in one colour, which becomes the text's own.
	svgPaint = regexp.MustCompile(`(fill|stroke)="#[0-9a-fA-F]{6}"`)
)

// take moves the SVG's glyphs to the shared set and sizes the drawing in
// ems, sunk below the line by its depth.
func (g *glyphs) take(svg string, baseline float64) string {
	for _, defs := range svgDefs.FindAllString(svg, -1) {
		for _, m := range svgSymbol.FindAllStringSubmatch(defs, -1) {
			if !g.seen[m[1]] {
				g.seen[m[1]] = true
				g.order = append(g.order, m[0])
			}
		}
	}
	svg = svgDefs.ReplaceAllString(svg, "")
	svg = strings.ReplaceAll(svg, ` xmlns:h5="http://www.w3.org/1999/xhtml"`, "")
	svg = svgPaint.ReplaceAllString(svg, `$1="currentColor"`)
	m := svgSize.FindStringSubmatch(svg)
	if m == nil {
		return svg
	}
	w, _ := strconv.ParseFloat(m[1], 64)
	h, _ := strconv.ParseFloat(m[2], 64)
	em := func(pt float64) string { return strconv.FormatFloat(pt/texPt, 'f', 3, 64) + "em" }
	return fmt.Sprintf(`<svg viewBox="0 0 %s %s" style="width:%s;height:%s;vertical-align:-%s" aria-hidden="true" focusable="false"`,
		m[1], m[2], em(w), em(h), em(h-baseline)) + svg[len(m[0]):]
}

func (g *glyphs) sprite() string {
	return `<svg class="tex-glyphs" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><defs>` + strings.Join(g.order, "") + "</defs></svg>\n"
}
