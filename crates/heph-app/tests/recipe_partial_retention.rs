//! Actual private112 original recipe claim + sealed PG + native Local custody.
//! No VM, provider absence or generic ownership adoption is asserted.

use hephaestus_app::recipe::partial_retention as coordinator;

#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Share frozen parsed/admission helpers; no fixture provider proof.
mod fixtures;
#[path = "recipe_partial_retention/private_migrations.rs"]
mod private_migrations;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Reuse actual identities, with role pools composed in this target.
mod support;

mod recipe_partial_retention {
    mod behavior;
    mod fixture;
    mod host;
    mod ledger;
    mod lineage;
    mod races;
    mod rollback;
    mod worker;
}
