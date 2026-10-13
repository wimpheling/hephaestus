//! Target-local actual113 plus first-birth lease exclusion, before any owned IO.
//! Existing frozen preparation/worker migration loaders remain unchanged.

use sqlx::migrate::Migrator;
use std::borrow::Cow;

pub fn checkpoint(version: i64) -> Migrator {
    assert_eq!(version, 113, "actual113 prerequisite only");
    let mut migrator = crate::base_schema::checkpoint(version);
    let mut migrations = migrator.iter().cloned().collect::<Vec<_>>();
    let prerequisite =
        include_str!("../../../../../../../migrations/0118_owned_first_ready_lease_exclusion.sql")
            .to_owned();
    let source = migrations
        .iter_mut()
        .find(|migration| migration.version == 113)
        .unwrap();
    *source = sqlx::migrate::Migration::new(
        113,
        source.description.clone(),
        source.migration_type,
        Cow::Owned(format!("{}\n{}", source.sql, prerequisite)),
        source.no_tx,
    );
    migrator.migrations = migrations.into();
    migrator
}
