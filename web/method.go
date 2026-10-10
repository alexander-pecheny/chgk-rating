package main

import (
	"bytes"
	_ "embed"
	"html"
	"regexp"
	"slices"
	"strings"

	"github.com/wyatt915/treeblood"
	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/extension"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/renderer"
	"github.com/yuin/goldmark/text"
	"github.com/yuin/goldmark/util"

	"ratingweb/ui"
)

// methodMD is the post on the rating, rendered once at start-up, so a
// formula that does not parse stops the server there.
//
//go:embed method.md
var methodMD []byte

// The post's own heading goes, since the page's title is the bar's last
// crumb, and so do the {llm}…{/llm} marks around prose the owner has not
// yet rewritten.
var (
	postHeading = regexp.MustCompile(`\A# [^\n]*\n`)
	llmMarks    = regexp.MustCompile(`\{/?llm\}`)
)

func renderMethod() (string, error) {
	src := llmMarks.ReplaceAll(postHeading.ReplaceAll(methodMD, nil), nil)
	md := goldmark.New(goldmark.WithExtensions(extension.Table, extension.Footnote, texMath{}))
	doc := md.Parser().Parse(text.NewReader(src))
	var all []*tex
	err := ast.Walk(doc, func(n ast.Node, entering bool) (ast.WalkStatus, error) {
		if t, ok := n.(interface{ formula() *tex }); ok && entering {
			all = append(all, t.formula())
		}
		return ast.WalkContinue, nil
	})
	if err != nil {
		return "", err
	}
	glyphs, err := typeset(all)
	if err != nil {
		return "", err
	}
	var out bytes.Buffer
	out.WriteString(glyphs)
	err = md.Renderer().Render(&out, src, doc)
	return out.String(), err
}

func (s *site) methodPage() (string, []ui.Item) {
	return S.Common.Nav.Method(), []ui.Item{s.modelLine(), ui.Article(ui.Raw(s.method))}
}

// modelLine names the version of the rules behind the numbers.
func (s *site) modelLine() ui.Node {
	if s.model == "" {
		return nil
	}
	return head(ui.Text(S.Common.ModelVersion(s.model)))
}

// texMath reads $…$ inside a line and a $$ … $$ block of its own lines as TeX.
type texMath struct{}

var (
	kindTex      = ast.NewNodeKind("Tex")
	kindTexBlock = ast.NewNodeKind("TexBlock")
	dollar       = []byte("$$")
)

// tex is one formula: its source, its pieces and their parts (splitDisplay),
// and the SVG of each part once typeset has set them.
type tex struct {
	src     string
	display bool
	pieces  [][]string
	svgs    [][]string
}

type texInlineNode struct {
	ast.BaseInline
	tex
}

func (n *texInlineNode) Kind() ast.NodeKind { return kindTex }
func (n *texInlineNode) formula() *tex      { return &n.tex }
func (n *texInlineNode) Dump(src []byte, level int) {
	ast.DumpHelper(n, src, level, map[string]string{"tex": n.src}, nil)
}

type texBlockNode struct {
	ast.BaseBlock
	tex
}

func (n *texBlockNode) Kind() ast.NodeKind { return kindTexBlock }
func (n *texBlockNode) formula() *tex      { return &n.tex }
func (n *texBlockNode) IsRaw() bool        { return true }
func (n *texBlockNode) Dump(src []byte, level int) {
	ast.DumpHelper(n, src, level, map[string]string{"tex": n.src}, nil)
}

func (texMath) Extend(m goldmark.Markdown) {
	// Ahead of emphasis, so the underscores of a subscript stay TeX.
	const first = 100
	m.Parser().AddOptions(
		parser.WithBlockParsers(util.Prioritized(texBlock{}, first)),
		parser.WithInlineParsers(util.Prioritized(texInline{}, first)),
	)
	m.Renderer().AddOptions(renderer.WithNodeRenderers(util.Prioritized(texRenderer{}, first)))
}

type texInline struct{}

func (texInline) Trigger() []byte { return []byte{'$'} }

func (texInline) Parse(_ ast.Node, block text.Reader, _ parser.Context) ast.Node {
	line, _ := block.PeekLine()
	end := bytes.IndexByte(line[1:], '$')
	if end <= 0 {
		return nil
	}
	block.Advance(end + 2)
	return &texInlineNode{tex: tex{src: strings.TrimSpace(string(line[1 : end+1]))}}
}

type texBlock struct{}

func (texBlock) Trigger() []byte { return []byte{'$'} }

func (texBlock) Open(_ ast.Node, r text.Reader, _ parser.Context) (ast.Node, parser.State) {
	line, _ := r.PeekLine()
	rest := bytes.TrimSpace(line)
	if !bytes.HasPrefix(rest, dollar) {
		return nil, parser.NoChildren
	}
	r.AdvanceToEOL()
	n := &texBlockNode{tex: tex{display: true}}
	rest = rest[len(dollar):]
	if body, ok := bytes.CutSuffix(rest, dollar); ok {
		n.src = strings.TrimSpace(string(body))
		return n, parser.Close
	}
	n.src = string(rest) + "\n"
	return n, parser.NoChildren
}

func (texBlock) Continue(node ast.Node, r text.Reader, _ parser.Context) parser.State {
	line, _ := r.PeekLine()
	r.AdvanceToEOL()
	n := node.(*texBlockNode)
	if body, ok := bytes.CutSuffix(bytes.TrimSpace(line), dollar); ok {
		n.src = strings.TrimSpace(n.src + string(body))
		return parser.Close
	}
	n.src += string(line)
	return parser.Continue | parser.NoChildren
}

func (texBlock) Close(ast.Node, text.Reader, parser.Context) {}
func (texBlock) CanInterruptParagraph() bool                 { return true }
func (texBlock) CanAcceptIndentedLine() bool                 { return false }

type texRenderer struct{}

func (texRenderer) RegisterFuncs(reg renderer.NodeRendererFuncRegisterer) {
	reg.Register(kindTex, func(w util.BufWriter, _ []byte, node ast.Node, entering bool) (ast.WalkStatus, error) {
		return writeTex(w, node.(*texInlineNode).formula(), entering)
	})
	reg.Register(kindTexBlock, func(w util.BufWriter, _ []byte, node ast.Node, entering bool) (ast.WalkStatus, error) {
		return writeTex(w, node.(*texBlockNode).formula(), entering)
	})
}

// writeTex shows Typst's drawing of the formula and keeps two texts behind
// it that nobody sees: the TeX, which is what a copy of the passage takes,
// and MathML, which is what a screen reader reads.
func writeTex(w util.BufWriter, t *tex, entering bool) (ast.WalkStatus, error) {
	if !entering {
		return ast.WalkSkipChildren, nil
	}
	mml, err := treeblood.TexToMML(strings.ReplaceAll(t.src, `\allowbreak`, ""), nil, t.display, false)
	if err != nil {
		return ast.WalkSkipChildren, err
	}
	// In a box of its own: a browser does not clip MathML's own children,
	// which would otherwise widen a phone's page.
	mml = `<span class="tex-mml">` + strings.TrimSpace(sortAttrs(mml)) + `</span>`
	src := html.EscapeString(t.src)
	if !t.display {
		_, err = w.WriteString(`<span class="tex">` + t.svgs[0][0] + `<span class="tex-src" aria-hidden="true">$` + src + `$</span>` + mml + `</span>`)
		return ast.WalkSkipChildren, err
	}
	var b strings.Builder
	b.WriteString(`<div class="tex-block"><div class="tex-row">`)
	for _, piece := range t.svgs {
		b.WriteString(`<span class="tex-piece">`)
		for _, svg := range piece {
			b.WriteString(`<span class="tex-part">` + svg + `</span>`)
		}
		b.WriteString(`</span>`)
	}
	b.WriteString(`</div><span class="tex-src" aria-hidden="true">$$` + src + `$$</span>` + mml + "</div>\n")
	_, err = w.WriteString(b.String())
	return ast.WalkSkipChildren, err
}

// treeblood writes a tag's attributes, and a style's declarations, in map order, which differs from run to
// run; sorted, the page is the same bytes on every start.
var (
	mmlTag  = regexp.MustCompile(`<([a-z]+)((?:\s+[a-z:-]+="[^"]*")+)>`)
	mmlAttr = regexp.MustCompile(`[a-z:-]+="[^"]*"`)
)

func sortAttrs(mml string) string {
	return mmlTag.ReplaceAllStringFunc(mml, func(tag string) string {
		m := mmlTag.FindStringSubmatch(tag)
		attrs := mmlAttr.FindAllString(m[2], -1)
		for i, a := range attrs {
			if css, ok := strings.CutPrefix(a, `style="`); ok {
				decls := strings.FieldsFunc(strings.TrimSuffix(css, `"`), func(r rune) bool { return r == ';' })
				slices.Sort(decls)
				attrs[i] = `style="` + strings.Join(decls, ";") + `;"`
			}
		}
		slices.Sort(attrs)
		return "<" + m[1] + " " + strings.Join(attrs, " ") + ">"
	})
}
