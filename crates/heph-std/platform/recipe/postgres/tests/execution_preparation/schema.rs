use std::borrow::Cow;

use sqlx::migrate::{Migration, MigrationType, Migrator};

pub fn checkpoint(version: i64) -> Migrator {
    assert!(matches!(version, 112 | 113), "explicit private target");
    assert_eq!(
        std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref(),
        Ok("1"),
        "private schema preflight requires explicit opt-in"
    );
    let base = crate::fixtures::private_migrations::checkpoint(107);
    let mut migrations = base.iter().cloned().collect::<Vec<_>>();
    let profile = [
        include_str!("../../../../../../../migrations/0108_run_volume_leases.sql"),
        include_str!("../../../../../../../migrations/0109_run_cleanup.sql"),
        include_str!("../../../../../../../migrations/0110_legacy_scalar_compatibility.sql"),
        include_str!("../../../../../../../migrations/0111_run_profile_lifecycle.sql"),
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>()
    .join("\n");
    migrations.push(Migration::new(
        108,
        Cow::Borrowed("private_profile_scalar_lifecycle"),
        MigrationType::Simple,
        Cow::Owned(profile),
        false,
    ));
    for (number, name, file) in [
        (
            109,
            "private_external_retirement",
            include_str!(
                "../../../../../../../migrations/0113_external_observation_retirement.sql"
            ),
        ),
        (
            110,
            "private_owned_registration",
            include_str!("../../../../../../../migrations/0114_owned_volume_registration.sql"),
        ),
        (
            111,
            "private_owned_backing",
            include_str!("../../../../../../../migrations/0115_owned_volume_backing.sql"),
        ),
        (
            112,
            "private_atomic_retention",
            include_str!("../../../../../../../migrations/0116_owned_partial_retention.sql"),
        ),
        (
            113,
            "private_execution_preparation",
            include_str!(
                "../../../../../../../migrations/0117_deployment_execution_preparation.sql"
            ),
        ),
    ] {
        if number <= version {
            migrations.push(Migration::new(
                number,
                Cow::Borrowed(name),
                MigrationType::Simple,
                Cow::Owned(file.to_owned()),
                false,
            ));
        }
    }
    Migrator {
        migrations: Cow::Owned(migrations),
        ..base
    }
}
