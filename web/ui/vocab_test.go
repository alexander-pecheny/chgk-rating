package ui

import (
	"encoding/json"
	"testing"
)

// The kit renders a primitive it has no expander for as nothing at all, so a
// primitive declared here and never wired up would vanish from every page
// without an error. This is the check that it cannot.
func TestEveryPrimitiveExpands(t *testing.T) {
	var v struct {
		Primitives []struct {
			Name string `json:"name"`
		} `json:"primitives"`
	}
	if err := json.Unmarshal(overlayVocab, &v); err != nil {
		t.Fatal(err)
	}
	for _, p := range v.Primitives {
		if _, ok := expanders[p.Name]; !ok {
			t.Errorf("%s has no expander", p.Name)
		}
	}
}
