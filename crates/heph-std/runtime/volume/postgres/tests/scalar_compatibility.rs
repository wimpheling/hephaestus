//! Private scalar compatibility: database lifecycle evidence, never physical absence.

#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Reuse the existing cross-project fixture type and graph.
mod support;
mod scalar_compatibility {
    mod fixture;
    mod guards;
    mod mutations;
    #[path = "../run_profiles_private/producers.rs"]
    mod producers;
    mod profiles;
    mod recovery;
}
