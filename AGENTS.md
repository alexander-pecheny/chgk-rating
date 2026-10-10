# Notes for agents

- Wrap every piece of prose you write or rewrite in `web/method.md`, the post the server renders at /method/, in `{llm}…{/llm}`, so the owner can find and rewrite it later. A marker may span several paragraphs, formulas included. The renderer drops the markers, so the page shows the text plain.
- The formulas in `web/method.md` are TeX that Typst draws as SVG when the server starts, reading it through the mitex package. A display formula breaks at a `\qquad` outside braces, with a gap, and at an `\allowbreak`, without one; put one wherever a formula is too wide for a phone.
