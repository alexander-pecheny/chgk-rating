package main

import (
	_ "embed"

	"ratingweb/ui"
)

// methodHTML is the post on the rating, docs/post.md, rendered with MathML by
// tools/method_html.py (`just method`).
//
//go:embed method.html
var methodHTML string

func methodPage() (string, []ui.Item) {
	return S.Common.Nav.Method(), []ui.Item{ui.Article(ui.Raw(methodHTML))}
}
