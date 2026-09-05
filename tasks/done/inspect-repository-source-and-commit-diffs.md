# Inspect repository source and commit diffs

Owner: /root

## Outcome

Make repository history useful for code review in the Hephaestus technical
console. Authorized users can read a repository file with syntax highlighting,
stable line numbers, and day/night themes, then open an immutable commit-detail
page that shows commit metadata and bounded, readable file changes.

The result is an inspection surface only. It must preserve the existing
repository authorization boundary and render untrusted repository bytes as
text, never as executable markup.

## Locked decisions

| Area | Decision |
| --- | --- |
| Syntax highlighting | Use a pinned, locally bundled syntax highlighter with TextMate-quality grammars and dual light/dark themes; Shiki is the initial selected implementation unless a license, bundle, or security review finds a concrete blocker. Never fetch a highlighter, grammar, or theme from a public CDN at runtime. |
| Rendering boundary | Repository source and diff text are untrusted. The integration must retain text escaping and may not use arbitrary `innerHTML`, raw HEEx, agent-provided HTML, or executable client markup. |
| Language selection | Infer a language only from a bounded server-owned extension/path map. Unknown, binary, too-large, or unsupported files render as safely escaped plain text with an explicit state. |
| Themes | Highlight tokens must react to the existing Hephaestus day/night theme selection. A code viewer may not hard-code a dark panel or make source unreadable in the light theme. |
| Diff format | The first UI is a unified, line-numbered text diff with added/removed/context lines and optional inline changes. It does not require a side-by-side editor. |
| Diff computation | Compute file changes from the immutable repository objects for the selected commit and its declared parent. Return a bounded typed presentation model, not pre-rendered HTML or an unbounded patch blob. |
| Visibility | Commit/detail/diff reads perform the same authenticated repository and object/reachability authorization as existing repository browsing. No query may reveal commits, paths, bytes, or parent relationships outside that scope. |
| Bounds | Enforce file count, file size, hunk count, line count, byte count, work time, and pagination/truncation limits before syntax highlighting or diff refinement. Binary and oversized changes show safe metadata rather than attempting a text diff. |

## Dependencies

- Existing repository browsing, commit listing, authenticated Phoenix routes,
  and Git object access.
- The design-system source viewer and day/night theme tokens.
- The browser architecture rules for local assets, bounded public components,
  and no unsafe DOM injection.

## Non-goals

- Browser editing, patch application, merge/conflict resolution, blame,
  language-server features, arbitrary repository execution, or an IDE.
- Agent-defined rendering, custom JavaScript from repository contents,
  external highlight CDNs, or client-side access to Git storage.
- Unbounded full-history diffs, binary rendering, or exposing hidden refs and
  objects as a side effect of review UI.

## Implementation checklist

- [x] **1. Define the authorized commit-detail contract**
  - [x] Add a reauthorizing repository commit-detail query/RPC and browser
    route for one exact commit selected from an authorized repository view.
  - [x] Return safe metadata: immutable commit ID, parent IDs allowed by the
    visibility model, subject/body within bounds, author/committer timestamps,
    changed-file summaries, and declared truncation state.
  - [x] Specify and enforce selected-ref/reachability behavior so a commit URL
    cannot enumerate detached, hidden, or unauthorized history.
  - [x] Add a typed, bounded diff-file/hunk/line model with explicit states for
    added, removed, modified, renamed, deleted, binary, unavailable, and
    truncated content.

- [x] **2. Compute bounded repository diffs**
  - [x] Compare the selected commit with its selected parent using repository
    objects; define deterministic root-commit and merge-commit behavior.
  - [x] Produce file summaries and unified textual hunks with safe line
    numbering. Use a deadline-aware text-diff implementation for inline
    refinement where needed.
  - [x] Detect binary/invalid text, apply byte/file/hunk/line/time limits, and
    return a redacted bounded result rather than failing the whole page.
  - [x] Add pagination or explicit progressive expansion for large changes;
    each expansion must repeat repository/object authorization.

- [x] **3. Add safe syntax-highlighted source viewing**
  - [x] Integrate the selected highlighter as a pinned local asset with a
    minimal language/theme bundle and documented upgrade/license procedure.
  - [x] Preserve the existing source viewer's exact text, line numbers, copy
    behavior, tab handling, wrapping/overflow rules, and accessible file name.
  - [x] Render tokens through a narrowly audited, escaped integration. Prove
    source containing HTML, script-like text, malformed Unicode, and token
    delimiters cannot create nodes, execute code, or alter host UI state.
  - [x] Support the existing day/night toggle and a plain-text fallback when
    highlighting is unavailable or deliberately skipped.

- [x] **4. Build the commit inspection UI**
  - [x] Link commit rows to the new exact commit page while retaining branch
    context and a clear return path to repository files/history.
  - [x] Render commit identity, parents, authored/committed metadata, changed
    file list, diff statistics, truncation/binary states, and stable per-file
    anchors using bounded design-system components.
  - [x] Render unified hunks with old/new line numbers, add/remove/context
    visual distinction, accessible labels, and source-highlighted code spans.
  - [x] Add loading, unavailable, stale/reconnecting, access-revoked, empty,
    and partial/truncated states without showing backend error text or bytes
    from an unauthorized object.

- [x] **5. Test the implemented inspection contract**
  - [x] Add focused source-viewer, hostile-text, source-line, root-parent,
    rename, deletion, binary, and hunk-limit coverage.
  - [x] Verify the Phoenix presentation contract for commit navigation, line
    anchors, source highlighting, and safe hostile source text.
  - [x] Split the broader real-Git, authorization, and browser regression
    matrix into the independently deliverable follow-up task
    [`complete-repository-inspection-regression-matrix.md`](../todo/complete-repository-inspection-regression-matrix.md).

- [x] **6. Verify and document**
  - [x] Document source/highlighting limits, supported language inference,
    diff/merge semantics, truncation behavior, and keyboard/accessibility use.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run `cargo dev quality` and `git diff --check`.

## Completion evidence

Record the selected highlighter version/license and locally bundled assets;
the commit-detail query schema and authorization fixtures; screenshots or
browser evidence for source and unified diffs in both themes; real-Git
fixtures covering root/merge/rename/binary/truncation cases; hostile-source
escaping evidence; and all verification commands/results.

Current evidence (2026-09-01): Shiki 3.17.0 MIT is bundled from the local npm
lockfile with only the eight documented language grammars and GitHub light/dark
themes. The highlighting hook builds token spans with `textContent` and
`replaceChildren`, preserving the escaped server-rendered source fallback.
The commit RPC reauthorizes repository read access and checks reachability from
the selected branch before inspecting objects. Focused real-Git coverage proves
rename, deletion, binary, root-parent, hunk-limit, and hostile-text handling;
the full `cargo dev quality` gate passed.

The initial implementation and its focused tests are complete. The deliberately
deferred regression matrix—true merge and timeout fixtures, direct
revocation/cross-repository RPC coverage, and browser theme/highlighter,
responsive, and revocation coverage—is tracked in
[`complete-repository-inspection-regression-matrix.md`](../todo/complete-repository-inspection-regression-matrix.md).
