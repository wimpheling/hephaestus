use release_service::{ActivateInstance, InstanceExecutionError};
use sqlx::{Postgres, Transaction};

use super::rows::{ActivationContext, database};

pub async fn reserve(
    tx: &mut Transaction<'_, Postgres>,
    context: &ActivationContext,
    command: ActivateInstance,
) -> Result<(), InstanceExecutionError> {
    let original: [u8; 32] = context
        .original_import_key
        .as_slice()
        .try_into()
        .map_err(|_| InstanceExecutionError::OutcomeUncertain)?;
    let mut keys = [
        i64::from_be_bytes(
            *original
                .first_chunk::<8>()
                .ok_or(InstanceExecutionError::OutcomeUncertain)?,
        ),
        i64::from_be_bytes(
            *command
                .command_key()
                .as_bytes()
                .first_chunk::<8>()
                .ok_or(InstanceExecutionError::OutcomeUncertain)?,
        ),
    ];
    keys.sort_unstable();
    for (index, key) in keys.into_iter().enumerate() {
        if index == 0 || key != keys[0] {
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(key)
                .execute(&mut **tx)
                .await
                .map_err(|error| database(&error))?;
        }
    }
    sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
        .bind(context.first_mutex)
        .bind(context.second_mutex)
        .execute(&mut **tx)
        .await
        .map_err(|error| database(&error))?;
    Ok(())
}
