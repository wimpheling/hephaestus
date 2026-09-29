# Purpose

`identity-application` connects verified login data to Hephaestus's internal
identity. It resolves an issuer and subject to an active user, gives the
caller a stable actor and idempotency identity, and manages browser sessions
from the same verified assertion.

# Responsibilities

After an external adapter verifies a sign-in, the caller passes the verified
issuer, subject, and bounded profile fields to `IdentityApplication`. It
constructs `VerifiedExternalIdentity` and delegates idempotent resolution to
`IdempotentIdentityResolver`, whose provider maps that pair to the corresponding
active Heph user. The result supplies one stable actor and idempotency identity,
so a retried login cannot accidentally become a different user.

The same boundary drives browser sessions. `BrowserSessionStore` creates a
session from the verified identity, checks an exact user and SID on each
request, and revokes that SID when the signed-in user logs out. Only a digest
of the request-only SID is retained by the store, and its results expose safe
metadata rather than bearer material.

# When

Use this boundary after middleware has verified a token or browser bootstrap
assertion. In the sign-in scenario, the handler passes the verified issuer and
subject plus the bounded profile fields to `IdentityApplication::resolve_identity`;
the returned `ResolvedIdentity` supplies the internal user and logical
mutation ID for the rest of the request. A session endpoint then sends the
verified issuer and subject with a fresh `BrowserSessionSid` to
`BrowserSessionStore::create_browser_session` and uses the returned metadata
for later session checks.
