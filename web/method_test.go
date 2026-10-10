package main

import (
	"strings"
	"testing"
)

func TestMethodRenders(t *testing.T) {
	html, err := renderMethod()
	if err != nil {
		t.Fatal(err)
	}
	for _, left := range []string{"$", "{llm}", "{/llm}", "merror"} {
		if strings.Contains(html, left) {
			t.Errorf("the rendered post still has %q", left)
		}
	}
	if !strings.Contains(html, `display="block"`) {
		t.Error("no display formula in the rendered post")
	}
}

func TestMethodIsStable(t *testing.T) {
	first, _ := renderMethod()
	for range 20 {
		if again, _ := renderMethod(); again != first {
			t.Fatal("two renders of the post differ")
		}
	}
}
