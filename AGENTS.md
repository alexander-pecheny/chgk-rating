# Notes for agents

- The live site's data lives in `/srv/a2` (on the /mnt volume), not in a home directory: the production checkout `~/new_chgk_rating` links `out` and `buff.db` there. Disk cleanups must leave `/srv/a2` alone; a worktree's own `out/` is scratch and may go.

- Wrap every piece of prose you write or rewrite in `web/method.md`, the post the server renders at /method/, in `{llm}…{/llm}`, so the owner can find and rewrite it later. A marker may span several paragraphs, formulas included. Never strip the markers, in the file or on the page: the renderer prints them, so prose the owner has not rewritten stays visibly marked.
- `config.toml`'s `model.version`, and a line under «## Изменения» in `web/method.md`, change only when the rules change: what is computed and from what. A technical change, such as speed, determinism or the order rows are read in, keeps the version even when it moves the last digits.
- The formulas in `web/method.md` are TeX that Typst draws as SVG when the server starts, reading it through the mitex package. A display formula breaks at a `\qquad` outside braces, with a gap, and at an `\allowbreak`, without one; put one wherever a formula is too wide for a phone.
