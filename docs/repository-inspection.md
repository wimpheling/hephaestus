# Repository source and commit inspection

Repository browsing renders only reachable Git objects that the authenticated
viewer may read. A commit URL is always evaluated again against its selected
branch; it cannot disclose detached, hidden, or cross-repository objects.

Source previews are limited to 1 MiB UTF-8 text. Binary, invalid UTF-8, and
oversized files remain unavailable rather than being decoded by the browser.
The source viewer preserves text escaping and line numbers. It uses the locally
bundled [Shiki](https://github.com/shikijs/shiki) 3.17.0 MIT package with the
Elixir, JSON, Markdown, Rust, shell, SQL, TOML, and YAML TextMate grammars and
GitHub light/dark themes. Tokens are added as browser text nodes; no repository
content or highlighter HTML is inserted with `innerHTML`.

Commit inspection compares the exact commit with a declared parent (the first
parent by default; the empty tree for a root commit). It pages changed files,
limits each response to 100 files, 50 hunks per file, 500 lines per hunk, and
128 KiB of textual diff per file. A binary, incomplete, or bounded change is
shown as metadata instead of an unbounded patch. Keyboard users can use the
standard browser find/copy commands on the semantic line-numbered source and
diff text.
