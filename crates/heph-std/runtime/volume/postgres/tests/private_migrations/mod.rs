//! Temporary explicit private-schema preflight; never an application migrator.
//!
//! Replace this staging loader with the embedded published migrator at release.

use sqlx::migrate::{Migration, MigrationType, Migrator};
use std::borrow::Cow;

pub fn checkpoint(version: i64) -> Migrator {
    // Temporary cross-package staging fixtures use SQLx's exact resolver;
    // replace with the embedded published migrator when this loader is removed.
    let mut migrations = sqlx::migrate::resolve_blocking(std::path::Path::new(
        "/home/a/projects/hephaestus/migrations",
    ))
    .expect("published migration directory")
    .into_iter()
    .map(|(migration, _)| migration)
    .filter(|entry| entry.version <= version)
    .collect::<Vec<_>>();
    if version > 107 {
        assert_eq!(
            std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref(),
            Ok("1"),
            "private schema preflight requires explicit opt-in"
        );
        for (number, name, files) in [
            (
                108,
                "private_run_volumes_and_cleanup",
                &[
                    "/var/tmp/0108_run_volume_leases.sql",
                    "/var/tmp/0108_run_cleanup_fragment.sql",
                ][..],
            ),
            (
                109,
                "private_external_observation_retirement",
                &["/var/tmp/heph-recipe-0109_external_observation_retirement.sql"][..],
            ),
            (
                110,
                "private_owned_volume_registration",
                &["/var/tmp/0110_owned_volume_registration.sql"][..],
            ),
            (
                111,
                "private_owned_volume_backing",
                &["/var/tmp/0111_owned_volume_backing.sql"][..],
            ),
            (
                112,
                "private_owned_partial_retention",
                &["/var/tmp/0112_owned_partial_retention.sql"][..],
            ),
        ] {
            if number <= version && !migrations.iter().any(|entry| entry.version == number) {
                let sql = files
                    .iter()
                    .map(|path| std::fs::read_to_string(path).expect("task-authored private SQL"))
                    .collect::<Vec<_>>()
                    .join("\n");
                migrations.push(Migration::new(
                    number,
                    Cow::Borrowed(name),
                    MigrationType::Simple,
                    Cow::Owned(sql),
                    false,
                ));
            }
        }
    }
    assert!(
        migrations.iter().any(|entry| entry.version == version),
        "exact requested checkpoint must exist"
    );
    migrations.sort_by_key(|entry| entry.version);
    Migrator {
        migrations: Cow::Owned(migrations),
        ..Migrator::DEFAULT
    }
}
