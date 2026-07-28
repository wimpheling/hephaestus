# Hephaestus authorization model

`hephaestus.fga` is the canonical production authorization model. Mélange
compiles it to PostgreSQL functions; production does not contact OpenFGA.

The toolchain is pinned to Mélange **v0.8.5**, upstream commit
`7b0ba2f0979cbb8ea1d83dbe0d7617535a0bac7d`. The Linux amd64 release archive
must match SHA-256
`6c4569544777bb5414532af8298098517a74e0afe39cef4a46f60c1cf8c9b051`.

Install and generate:

```sh
scripts/install-melange.sh
scripts/generate-authz.sh --write
```

CI checks drift by running the same generator without `--write`:

```sh
scripts/generate-authz.sh
HEPHAESTUS_POSTGRES_TEST_URL='postgres://...?sslmode=disable' \
  scripts/check-authz.sh
```

The exact generation command is:

```sh
melange generate migration \
  --schema authz/hephaestus.fga \
  --up \
  --no-update-check > migrations/0003_melange_generated.sql
```

`0004_rls_and_roles.sql` wraps the generated dispatcher with a locked-down
security-definer function. It does not modify generated SQL. This lets RLS call
the generated evaluator without recursively applying tenant policies to tuple
source tables.

OpenFGA is a compatibility-test oracle only. It is never a runtime dependency.
The compatibility fixtures are evaluated with the pinned OpenFGA CLI v0.7.19:

```sh
scripts/install-openfga-cli.sh
scripts/check-openfga-model.sh
scripts/check-openfga-service.sh
```

The service check starts OpenFGA v1.15.1 pinned by OCI digest, imports the
canonical model and equivalent tuples, runs all 46 decisions, and removes the
ephemeral instance. It has no production deployment path.
