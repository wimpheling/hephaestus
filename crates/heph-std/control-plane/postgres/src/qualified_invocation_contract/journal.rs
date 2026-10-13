use super::{Postgres, Transaction, invalid};
use sqlx::{FromRow, migrate::Migrator};

static CANONICAL: Migrator = sqlx::migrate!("../../../../migrations");
const MIGRATION_COUNT: usize = 143;

pub(super) fn validate_inventory() -> Result<(), sqlx::Error> {
    if CANONICAL.iter().count() != MIGRATION_COUNT {
        return Err(invalid(
            "compiled canonical migration inventory must contain exactly 143 entries",
        ));
    }
    for (index, migration) in CANONICAL.iter().enumerate() {
        let version = i64::try_from(index + 1)
            .map_err(|_| invalid("compiled migration version is unrepresentable"))?;
        if migration.version != version
            || migration.migration_type != sqlx::migrate::MigrationType::Simple
            || migration.no_tx
            || migration.checksum.len() != 48
        {
            return Err(invalid(
                "compiled canonical migration inventory must be transactional 1..=143",
            ));
        }
    }
    Ok(())
}

#[derive(FromRow)]
struct JournalEntry {
    version: i64,
    description: String,
    success: bool,
    checksum: Vec<u8>,
}

pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let genuine: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pg_catalog.pg_class AS relation
            JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname = 'public' AND relation.relname = '_sqlx_migrations'
              AND relation.relkind = 'r' AND NOT relation.relrowsecurity
              AND pg_catalog.pg_get_userbyid(relation.relowner) = 'postgres'
        )",
    )
    .fetch_one(&mut **admin)
    .await?;
    if !genuine {
        return Err(invalid(
            "canonical journal must be the owner-controlled public table",
        ));
    }
    let entries: Vec<JournalEntry> = sqlx::query_as(
        "SELECT version, description, success, checksum
         FROM public._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut **admin)
    .await?;
    if entries.len() != MIGRATION_COUNT {
        return Err(invalid(
            "observed journal must contain exactly the 143 compiled migrations",
        ));
    }
    for (entry, migration) in entries.iter().zip(CANONICAL.iter()) {
        if entry.version != migration.version
            || entry.description != migration.description
            || !entry.success
            || entry.checksum.as_slice() != migration.checksum.as_ref()
        {
            return Err(invalid(
                "observed journal version, description, success or SHA384 differs",
            ));
        }
    }
    Ok(())
}
