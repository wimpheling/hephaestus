//! Target-only atomic proposal; preserves the verified worker staging loader.

use crate::fixtures::private_migrations as baseline;

use sqlx::migrate::{Migration, MigrationType, Migrator};
use std::borrow::Cow;

pub fn checkpoint(version: i64) -> Migrator {
    assert_eq!(version, 112, "this target verifies the atomic 112 proposal");
    let baseline = baseline::checkpoint(111);
    let mut migrations = baseline.iter().cloned().collect::<Vec<_>>();
    migrations.push(Migration::new(
        112,
        Cow::Borrowed("private_atomic_partial_retention"),
        MigrationType::Simple,
        Cow::Owned(
            include_str!("../../../../migrations/0116_owned_partial_retention.sql").to_owned(),
        ),
        false,
    ));
    Migrator {
        migrations: Cow::Owned(migrations),
        ..baseline
    }
}
