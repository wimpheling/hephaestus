# Reference session-chat browser UI

This directory is a release-owned browser adapter for the protocol in the
parent example. It uses `isomorphic-git` 1.42.2 and `@isomorphic-git/lightning-fs`
4.9.0. The browser talks only to the installed UI origin at
`/_heph/git/<repository UUID>`; the HttpOnly child cookie is sent by `fetch`
with `credentials: include`, and the adapter never reads it or creates an
`Authorization` header.

The release installer must copy `index.html`, `style.css`, and the generated
`dist/session-chat-ui.js` into its static UI artifact. The authenticated host
bootstrap must provide the repository UUID as immutable route metadata at
`/_heph/ui-context` with same-origin credentials. The authenticated host must
return exactly `{"repository_id":"<canonical UUID>"}` after rechecking the
live child session, generation, installation target, and release-declared Git
access. The static artifact is immutable; it never reads a global variable or
URL-selected repository. The host's
`Heph-Git-Actor-Id` response header is the only actor identity displayed by
the UI; it is not accepted as a request header.

When a release-owned session repository is empty, this UI initializes it with
one ordinary `main` commit containing the v1 manifest and release-owned
participant declarations, including the verified human participant. The Git
principal remains the authenticated human; protocol actor fields are release
presentation claims. Fork/new-session creation remains a release composition
operation.

The release source snapshot includes the generated `dist/session-chat-ui.js`
and `dist/session-chat-ui.css` files. The release build only copies those
checked-in assets, so its disabled-network builder does not need Node, npm, or
an npm cache. Regenerate and verify the snapshot with the locked toolchain:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm run check
```

For a disabled-network verification, use `npm ci --offline` when the pinned
packages are already in the local npm cache. Do not retain `node_modules` or
package archives in the release source tree. Production VM/browser checks must
exercise the actual installed origin and Git service separately.
