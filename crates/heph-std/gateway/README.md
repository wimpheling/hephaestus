# Purpose

This directory contains the standard gateway providers. PostgreSQL installs and
authorizes immutable gateway declarations and service state, while the edge
adapter reconciles those declarations into a private Caddy boundary and
dispatches bounded requests.

# Responsibilities

The providers carry route revisions, runtime authority, mailbox publications,
service ownership, logs, and failure evidence from durable state to the edge.
The edge side validates routes and request limits, preserves exposure rules,
and keeps Caddy administration and service traffic on private paths.

# When

Use the PostgreSQL provider when accepting a repository gateway declaration or
coordinating service instances. Feed its active route and desired
configuration results to the edge provider, then run reconciliation and request
dispatch through the private Caddy and VM interfaces.
