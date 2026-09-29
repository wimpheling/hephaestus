//! Real `PostgreSQL` coverage for gateway admission-mode selection and cleanup.

#[path = "service_acceptance/admission.rs"]
mod admission;
#[path = "service_acceptance/exposure.rs"]
mod exposure;
#[path = "service_acceptance/lifecycle.rs"]
mod lifecycle;
#[path = "service_acceptance/support_database.rs"]
mod support_database;
#[path = "service_acceptance/support_fixture.rs"]
mod support_fixture;
#[path = "service_acceptance/support_types.rs"]
mod support_types;
