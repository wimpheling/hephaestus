# Purpose

This directory contains concrete identity providers for the provider-neutral
authentication contracts. It verifies external OIDC assertions, maps verified
issuer and subject pairs to active Heph users, persists browser sessions, and
supports the local Git credential-helper protocol for developer tokens.

# Responsibilities

The OIDC adapter validates signed claims and interactive-flow state inputs. The
PostgreSQL adapter performs idempotent identity mapping and browser-session
lifecycle operations, while the Git helper keeps developer credentials in
authority-scoped private files.

# When

Use these adapters while composing inbound authentication and developer Git
access. Verify an external assertion before mapping it through PostgreSQL, use
the session store for browser cookies and logout, and install the helper where
Git invokes the local credential protocol.
