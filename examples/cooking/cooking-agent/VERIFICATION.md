# Local deterministic verification (2026-09-05)

Gateway: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features`,
`cargo test --all-features` (4 passed), `cargo doc --all-features --no-deps`.
Clippy pedantic is denied in the application manifest, without exceptions.

Cooking/relay/blog: `python3 -m unittest -v` in cooking-agent; 9 deterministic
tests plus one optional real Hugo build. The tests cover bounded authenticated
input, normalized identity, framed host broker protocol, SQLite duplicate/conflict
handling, lost relay response, fresh database connection and fresh workspace
restart, old-event context regression, safe Unicode Markdown, v2 transactional
migration/re-entry and deliberate rollback.

Actual Hugo HTML generation was run successfully using official Hugo v0.150.1
linux-amd64, from the GitHub gohugoio/hugo release. The downloaded archive SHA-256
matched the official release checksums file:

`c91fbd9d47c87d604a831b9ea86aa54009a686485df5f8b1d39758fddd5e5b09`

Command: `COOKING_HUGO_BINARY=/tmp/cooking-hugo.RByPfB/hugo python3 -m unittest -v`
(10 passed). The temporary binary path is evidence for this machine, not a
release dependency. Reproduction should acquire that exact official version,
verify its checksum, then supply its local executable path. All generated HTML,
work trees and SQLite files were contained in test-owned temporary directories.

Both agent.toml files and gateway heph.gateways.toml were parsed by the current
Hephaestus agent_config Rust parser without diagnostics. This verifies actual
platform manifest compatibility, beyond plain TOML syntax.

These results establish local application behavior only. The platform's joined
PostgreSQL/NATS/Caddy/libkrun fixture separately proves installed release execution,
broker substitution, mailbox delivery and controlled result/provenance. Local
tests do not establish release build publication provenance or the remaining
complete MVP-05 acceptance matrix. Real Telegram delivery and public deployment
are outside MVP-05 scope.

## Joined installed slice

The Hephaestus cooking runner (now `examples/cooking/run.sh`) completed successfully on
2026-09-05 at 14:18 UTC: one golden journey passed, four gateway PostgreSQL
regressions passed, and runtime/cgroup cleanup was verified. It exercised real
Caddy ingress, PostgreSQL/NATS delivery, the compiled released Rust gateway,
the exact Python cooking artifact inside libkrun, host-only HTTPS credential
substitution to deterministic TLS model/relay endpoints, SQLite state, controlled
Git result creation and authorized/unauthorized provenance inspection.

Redacted identifiers from that disposable fixture:

| Record | ID |
| --- | --- |
| Mailbox | 36441a8b-729a-4561-917e-4b9201e1711d |
| Cooking run | 417922be-fb60-49d2-9962-37a32db83092 |
| Cooking revision | ddf12f57-967c-4e74-8769-f0f4e1defa0c |
| State lease | e346d3ac-c8f3-4a89-8d27-7e4ea022f6e6 |
| Result commit | 1ba8649a6dcf664820dd2d51c0af63498690e20d |

The reviewed runtime image was
`localhost/hephaestus/python-ubuntu@sha256:cf5f70330594d10e4445178f1371ca63fd8e9e4dae54ddd78612796288c26ae9`.
The fixture seeds installation/release records and validates exact imported
artifacts; it does not replace isolated application build/publication evidence.
The broader concurrent/update/crash acceptance matrix remains outside this
one-request proof. Real Telegram integration is excluded from MVP-05.

This journey also exposed two generic target-resolution omissions: mailbox runs
had no Git receive run_request, so runtime context and workspace lookup failed
to use their captured mailbox-attempt commit. The production adapters now resolve
that exact snapshot. A separate real-PostgreSQL/Git regression advances the live
ref, verifies both adapters retain the older target, prepares and completes its
workspace, and proves one review proposal with that exact target on retry. This
test failed before the mailbox-proposal trigger fix and passes with migration 62.
