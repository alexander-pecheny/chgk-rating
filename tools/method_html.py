# /// script
# dependencies = ["latex2mathml==3.78.0", "markdown-it-py==3.0.0", "mdit-py-plugins==0.4.2"]
# ///
"""The methodology page from the post: Markdown in, an HTML fragment with
MathML out, which the server embeds at /method/.

    uv run tools/method_html.py docs/post.md web/method.html
"""

import re
import sys

from latex2mathml.converter import convert
from markdown_it import MarkdownIt
from mdit_py_plugins.dollarmath import dollarmath_plugin
from mdit_py_plugins.footnote import footnote_plugin


def math(content, display):
    return convert(content.strip(), display="block" if display else "inline")


def render(md_text):
    md = MarkdownIt("commonmark", {"html": False}).enable("table")
    md.use(footnote_plugin).use(dollarmath_plugin, allow_space=True, double_inline=True)
    md.add_render_rule("math_inline", lambda self, tokens, idx, options, env: math(tokens[idx].content, False))
    md.add_render_rule("math_block", lambda self, tokens, idx, options, env: math(tokens[idx].content, True) + "\n")
    # The page's title is the bar's last crumb, so the post's own heading goes.
    body = re.sub(r"\A# [^\n]*\n", "", md_text)
    # {llm}…{/llm} marks prose the owner has not yet rewritten; the page shows it plain.
    body = re.sub(r"\{/?llm\}", "", body)
    # A footnote cited twice is still one note: [3], not [3:1].
    return re.sub(r'(<a href="#fn\d+" id="fnref[\d:]+">)\[(\d+)(?::\d+)?\]', r"\1\2", md.render(body))


def main():
    src, out = sys.argv[1], sys.argv[2]
    with open(src, encoding="utf-8") as f:
        html = render(f.read())
    with open(out, "w", encoding="utf-8") as f:
        f.write(html)


if __name__ == "__main__":
    main()
