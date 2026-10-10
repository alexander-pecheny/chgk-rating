package main

import (
	"regexp"
	"slices"
	"strings"
	"testing"
)

var texSrc = regexp.MustCompile(`<span class="tex-src"[^>]*>[^<]*</span>`)

func TestMethodRenders(t *testing.T) {
	html, err := renderMethod()
	if err != nil {
		t.Fatal(err)
	}
	// The TeX stays only in the hidden text a copy takes.
	shown := texSrc.ReplaceAllString(html, "")
	for _, left := range []string{"$", "{llm}", "{/llm}", "merror"} {
		if strings.Contains(shown, left) {
			t.Errorf("the rendered post still has %q", left)
		}
	}
	for _, want := range []string{`display="block"`, `class="tex-piece"`, `<symbol id=`, "currentColor", `$$\tilde g_i =`} {
		if !strings.Contains(html, want) {
			t.Errorf("the rendered post has no %q", want)
		}
	}
}

func TestMethodIsStable(t *testing.T) {
	first, _ := renderMethod()
	for range 2 {
		if again, _ := renderMethod(); again != first {
			t.Fatal("two renders of the post differ")
		}
	}
}

func TestSplitDisplay(t *testing.T) {
	got := splitDisplay(`a, \qquad \frac{b \qquad c}{d} \allowbreak \cdot e \qquad x = \begin{cases} 1 \qquad 2 \end{cases} \qquadx`)
	want := [][]string{{`a,`}, {`\frac{b \qquad c}{d}`, `\mathord{} \cdot e`}, {`x = \begin{cases} 1 \qquad 2 \end{cases} \qquadx`}}
	if !slices.EqualFunc(got, want, slices.Equal) {
		t.Errorf("got %q, want %q", got, want)
	}
}
