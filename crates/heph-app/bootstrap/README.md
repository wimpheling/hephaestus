# Purpose

`bootstrap-postgres` provides the trusted operator and end-to-end seed
boundaries used to prepare a Hephaestus installation. It creates deterministic
projects, repositories, identities, releases, registry records, and related
database fixtures so a fresh environment can be exercised consistently.

# Responsibilities

The bootstrap binaries assemble the same domain and PostgreSQL adapters used by
the application, validate their inputs, and perform repeatable seed operations.
The operator binary is the explicit administrative entry point; the E2E seed
binary prepares the data required by integration scenarios. Seed identities,
repository ownership, release metadata, and registry publication state are
created through their owning services so later requests observe valid
provenance and authorization relationships.

# When

Run `hephaestus-operator` for deliberate installation administration and
`hephaestus-e2e-seed` when preparing a repeatable integration environment. The
application startup path should use the resulting records rather than
recreating them ad hoc.
