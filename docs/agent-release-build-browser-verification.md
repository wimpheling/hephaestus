# Agent release/build browser verification

This document records the browser verification boundary for the agent
release/build journey. It is intentionally separate from the task checklist:
the checklist remains the source of task state, while this file records the
evidence and the limits of the available runner.

## Runner

The repository has a real Chromium runner at [`e2e/playwright`](../e2e/playwright)
and a full-stack launcher at [`scripts/run-ui-e2e.sh`](../scripts/run-ui-e2e.sh).
The launcher starts isolated PostgreSQL, NATS, OIDC, the Rust daemon, and
Phoenix, then runs Playwright against the resulting application. It also
checks Phoenix storage isolation, secret-sentinel leakage, JetStream storage,
and ephemeral secret cleanup.

Run it locally with:

```sh
HEPHAESTUS_PLAYWRIGHT_SKIP_BROWSER_INSTALL=1 scripts/run-ui-e2e.sh
```

The skip flag is appropriate when Chromium is already cached. Omit it on a
fresh runner.

## Covered browser journey

The Playwright suite now exercises, in order:

1. draft release review, version submission, and publication through the real
   LiveView controls and ReleaseService RPCs;
2. repository Builds navigation, build history, build detail, bounded-log
   empty state, and release provenance;
3. the existing live-review, secret, instance, run, update, recovery, focus,
   accessibility, and non-disclosure journeys.

The first two tests are serial with the existing suite because publication is
a durable fixture transition required by the later instance tests.

## Fixture workflow audit

The browser fixture still has one deliberate infrastructure boundary. The
seed uses the trusted forge application operations for the organization,
project, and repository, but directly prepares completed `build_requests`,
artifact rows, release-agent rows, and draft release rows. The browser then
publishes every seeded draft through the normal authenticated application
workflow. Consequently, the final `published` state is produced by the
ReleaseService mutation, committed authorization/outbox path, and the
browser-visible UI rather than by fixture SQL.

The build portion cannot currently be replaced by the same durable workflow:
the browser stack starts `hephaestusd` but does not start a build worker, and
the exposed BuildService has typed request/list/get plus action/watch/log
contracts without a browser workflow that executes the isolated build, imports
its manifest, and calls the trusted `complete_build` operation. Removing the
prepared build/artifact records would therefore make the release journey
impossible rather than test the product workflow. This is the exact remaining
fixture blocker.

## Evidence

On 2026-08-01, with Podman, Node/npm, cached Chromium, PostgreSQL, NATS, and
the Phoenix image available:

```text
HEPHAESTUS_PLAYWRIGHT_SKIP_BROWSER_INSTALL=1 scripts/run-ui-e2e.sh: exit 0
Phoenix isolation verified
6 Playwright tests: passed
```

An earlier post-change attempt exited `101` before Playwright started because
the new build-action error variants had not yet been reconciled in a release
error mapper. That compile issue has since been repaired and the Rust quality
gates are green. The remaining browser limitation is the fixture workflow
described above: it still cannot execute a real isolated build and import its
manifest without a build worker.
