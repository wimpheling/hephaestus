//! Validation of explicit gateway configuration secret selections.

use super::{
    ConfigureRevisionRow, ConfigureRouteRow, GatewayConfigureError, GatewayManagementError,
    GatewaySecretSelection, PostgresGatewayManagement, valid_gateway_slot, valid_header_name,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::AuthenticatedIdentity;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;

pub async fn validate_secret_selections(
    management: &PostgresGatewayManagement,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    current: &ConfigureRevisionRow,
    routes: &[ConfigureRouteRow],
    selections: &[GatewaySecretSelection],
) -> Result<(), GatewayConfigureError> {
    let mut slots = BTreeSet::new();
    for selection in selections {
        if !valid_gateway_slot(&selection.slot_key)
            || !slots.insert(selection.slot_key.clone())
            || selection.route_path.is_empty()
            || !valid_header_name(&selection.header_name)
        {
            return Err(GatewayConfigureError::InvalidArgument);
        }
        if !current
            .secret_slots
            .iter()
            .any(|slot| slot == &selection.slot_key)
        {
            return Err(GatewayConfigureError::InvalidArgument);
        }
        let route = routes
            .iter()
            .find(|route| route.path == selection.route_path);
        if route.is_none() {
            return Err(GatewayConfigureError::InvalidArgument);
        }
        management
            .require(
                tx,
                identity,
                Permission::BindBrokered,
                ObjectRef::new(ObjectType::SecretImport, selection.import_id),
            )
            .await
            .map_err(|error| match error {
                GatewayManagementError::Persistence(error) => {
                    GatewayConfigureError::Persistence(error)
                }
                _ => GatewayConfigureError::Denied,
            })?;
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS (
           SELECT 1 FROM secret_imports imported
           JOIN secret_grants granted ON granted.id = imported.grant_id
           JOIN secrets owned ON owned.id = imported.secret_id
           JOIN secret_versions version ON version.id = $2 AND version.secret_id = owned.id
           WHERE imported.id = $1 AND imported.target_kind = 'project'
             AND imported.target_id = $3 AND imported.status = 'active'
             AND granted.status = 'active' AND owned.status = 'active'
             AND version.status = 'active' AND version.revoked_at IS NULL
             AND version.purged_at IS NULL
             AND 'normal' = ANY(granted.phases)
             AND 'brokered' = ANY(granted.delivery_modes)
             AND 'brokered' = ANY(owned.allowed_delivery_modes)
             AND cardinality(granted.destinations) = 0
         )",
        )
        .bind(selection.import_id)
        .bind(selection.secret_version_id)
        .bind(current.project_id)
        .fetch_one(&mut **tx)
        .await?;
        if !valid {
            return Err(GatewayConfigureError::NotFound);
        }
    }
    Ok(())
}
