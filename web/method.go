package main

import (
	"bytes"
	_ "embed"
	"regexp"

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

// methodMD is the post on the rating, rendered with its formulas as MathML
// once at start-up, so a formula that does not parse stops the server there.
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
	var out bytes.Buffer
	err := md.Convert(src, &out)
	return out.String(), err
}

func (s *site) methodPage() (string, []ui.Item) {
	return S.Common.Nav.Method(), []ui.Item{ui.Article(ui.Raw(s.method))}
}

// texMath reads $…$ inside a line and a $$ … $$ block of its own lines as TeX.
type texMath struct{}

var (
	kindTex      = ast.NewNodeKind("Tex")
	kindTexBlock = ast.NewNodeKind("TexBlock")
	dollar       = []byte("$$")
)

type texInlineNode struct {
	ast.BaseInline
	tex []byte
}

func (n *texInlineNode) Kind() ast.NodeKind { return kindTex }
func (n *texInlineNode) Dump(src []byte, level int) {
	ast.DumpHelper(n, src, level, map[string]string{"tex": string(n.tex)}, nil)
}

type texBlockNode struct {
	ast.BaseBlock
	tex []byte
}

func (n *texBlockNode) Kind() ast.NodeKind { return kindTexBlock }
func (n *texBlockNode) IsRaw() bool        { return true }
func (n *texBlockNode) Dump(src []byte, level int) {
	ast.DumpHelper(n, src, level, map[string]string{"tex": string(n.tex)}, nil)
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
	return &texInlineNode{tex: bytes.Clone(line[1 : end+1])}
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
	n := &texBlockNode{}
	rest = rest[len(dollar):]
	if body, ok := bytes.CutSuffix(rest, dollar); ok {
		n.tex = bytes.Clone(body)
		return n, parser.Close
	}
	n.tex = append(bytes.Clone(rest), '\n')
	return n, parser.NoChildren
}

func (texBlock) Continue(node ast.Node, r text.Reader, _ parser.Context) parser.State {
	line, _ := r.PeekLine()
	r.AdvanceToEOL()
	n := node.(*texBlockNode)
	if body, ok := bytes.CutSuffix(bytes.TrimSpace(line), dollar); ok {
		n.tex = append(n.tex, body...)
		return parser.Close
	}
	n.tex = append(n.tex, line...)
	return parser.Continue | parser.NoChildren
}

func (texBlock) Close(ast.Node, text.Reader, parser.Context) {}
func (texBlock) CanInterruptParagraph() bool                 { return true }
func (texBlock) CanAcceptIndentedLine() bool                 { return false }

type texRenderer struct{}

func (texRenderer) RegisterFuncs(reg renderer.NodeRendererFuncRegisterer) {
	reg.Register(kindTex, func(w util.BufWriter, _ []byte, node ast.Node, entering bool) (ast.WalkStatus, error) {
		return writeTex(w, node.(*texInlineNode).tex, false, entering)
	})
	reg.Register(kindTexBlock, func(w util.BufWriter, _ []byte, node ast.Node, entering bool) (ast.WalkStatus, error) {
		return writeTex(w, node.(*texBlockNode).tex, true, entering)
	})
}

func writeTex(w util.BufWriter, tex []byte, display, entering bool) (ast.WalkStatus, error) {
	if !entering {
		return ast.WalkSkipChildren, nil
	}
	mml, err := treeblood.TexToMML(string(bytes.TrimSpace(tex)), nil, display, false)
	if err == nil {
		_, err = w.WriteString(mml)
	}
	return ast.WalkSkipChildren, err
}
