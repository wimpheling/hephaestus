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
        "/var/tmp/0108_run_volume_leases.profile.sql",
        "/var/tmp/0108_run_cleanup_fragment.profile.sql",
        "/var/tmp/0108_legacy_scalar_compatibility.sql",
        "/var/tmp/0108_run_profile_lifecycle.sql",
    ]
    .into_iter()
    .map(|path| std::fs::read_to_string(path).expect("frozen private profile SQL"))
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
            "/var/tmp/heph-recipe-0109_external_observation_retirement.sql",
        ),
        (
            110,
            "private_owned_registration",
            "/var/tmp/0110_owned_volume_registration.sql",
        ),
        (
            111,
            "private_owned_backing",
            "/var/tmp/0111_owned_volume_backing.sql",
        ),
        (
            112,
            "private_atomic_retention",
            "/var/tmp/0112_owned_partial_retention.atomic.sql",
        ),
        (
            113,
            "private_execution_preparation",
            "/var/tmp/0113_deployment_execution_preparation.sql",
        ),
    ] {
        if number <= version {
            migrations.push(Migration::new(
                number,
                Cow::Borrowed(name),
                MigrationType::Simple,
                Cow::Owned(std::fs::read_to_string(file).expect("frozen private proposal")),
                false,
            ));
        }
    }
    Migrator {
        migrations: Cow::Owned(migrations),
        ..base
    }
}
