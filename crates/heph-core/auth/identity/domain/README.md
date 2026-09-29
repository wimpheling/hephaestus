# Purpose

`identity-domain` gives every authenticated request the same actor and
provenance shape, then supplies the values needed to create and validate a
browser session. These values let application, authorization, and audit code
refer to a user without depending on one identity provider's representation.

# Responsibilities

Once sign-in has mapped an external issuer and subject to a Heph user,
`AuthenticatedIdentity` carries that internal actor together with the verified
claims, request correlation, and actor-bound idempotency ID. Authorization and
application code can therefore make decisions about the same user throughout
the request, including when a mutation is retried.

For browser access, the domain represents the SID, its session metadata,
bounded lifetime, and revocation reason. `browser_session_sid_digest` lets a
store find the exact session without retaining the bearer, while
`browser_session_identity_binding_digest` binds that session to the verified
user. `Debug` and `Display` redact the SID; callers use
`BrowserSessionSid::to_protocol_string()` only at the deliberate cookie or
mediator serialization boundary.

# When

Use these values after an authentication provider has verified and mapped the
caller. The application constructs `AuthenticatedIdentity` with the internal
user, issuer, subject, claims, and request ID, then may replace its default
idempotency ID with the actor-bound mutation ID before passing it to
authorization or persistence. A session store uses the domain digest helpers
when creating and checking the browser session.
