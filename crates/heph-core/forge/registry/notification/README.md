# Purpose

`registry-notification` converts a Zot CloudEvent callback into a bounded,
authenticated observation that Forge can reconcile. The result identifies the
observed repository path, event action, immutable reference, source event, and
payload digest for durable inbox processing.

# Responsibilities

`CallbackCredential` authenticates the configured callback secret, while
`parse_notification` validates the binary-mode CloudEvent envelope, event type,
timestamp, canonical repository path, and payload shape. The parser derives a
stable `NotificationIdempotencyKey` and hashes the body so repeated or altered
callbacks can be recognized without treating a callback as publication proof.

Errors remain typed and bounded, and the observation exposes metadata rather
than an arbitrary callback payload. Registry reconciliation performs the
authoritative read from Zot before changing publication state.

# When

Use this crate in the registry HTTP callback handler. Authenticate the
presented credential, call `parse_notification` with the request headers and
body, and enqueue the resulting `NotificationObservation` for the
`registry-reconciler` workflow.
