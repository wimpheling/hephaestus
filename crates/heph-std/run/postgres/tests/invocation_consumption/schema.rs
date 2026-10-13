//! Target-local corrected108 with Legacy metadata BEFORE Invocation115.
//! Original shared migration loaders and every source SQL file stay unchanged.

use sqlx::migrate::{Migration, Migrator};

pub fn checkpoint(version: i64) -> Migrator {
    let mut migrator = crate::prior_schema::checkpoint(version);
    let mut rows = migrator.iter().cloned().collect::<Vec<_>>();
    let source = rows.iter_mut().find(|row| row.version == 108).unwrap();
    let placement =
        include_str!("../../../../../../migrations/0112_legacy_vm_placement.sql").to_owned();
    *source = Migration::new(
        108,
        source.description.clone(),
        source.migration_type,
        format!("{}\n{}", source.sql, placement).into(),
        source.no_tx,
    );
    migrator.migrations = rows.into();
    migrator
}
