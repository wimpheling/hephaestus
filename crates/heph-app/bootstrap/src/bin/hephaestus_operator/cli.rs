use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::error::Error;
use uuid::Uuid;

pub async fn execute(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage().into());
    };
    match command {
        "inspect-release" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            super::inspection::inspect(
                pool,
                &identity,
                super::inspection::InspectionTarget::Release,
                object_id,
            )
            .await
        }
        "inspect-instance" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            super::inspection::inspect(
                pool,
                &identity,
                super::inspection::InspectionTarget::AgentInstance,
                object_id,
            )
            .await
        }
        "inspect-secret" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            super::inspection::inspect(
                pool,
                &identity,
                super::inspection::InspectionTarget::Secret,
                object_id,
            )
            .await
        }
        "metrics" => {
            let identity = identity_argument(arguments, 1, Some(2))?;
            super::inspection::inspect_metrics(pool, &identity).await
        }
        "recover-update" => super::recovery::recover_update(pool, arguments).await,
        "abandon-build" => super::recovery::abandon_build(pool, arguments).await,
        "provision-image-catalog" => super::catalog::provision_image_catalog(pool, arguments).await,
        "registry-retention-report" => {
            super::catalog::registry_retention_report(pool, arguments).await
        }
        _ => Err(usage().into()),
    }
}

pub fn inspection_arguments(
    arguments: &[String],
) -> Result<(AuthenticatedIdentity, Uuid), Box<dyn Error>> {
    Ok((
        identity_argument(arguments, 1, Some(3))?,
        Uuid::parse_str(argument(arguments, 2)?)?,
    ))
}

pub fn identity_argument(
    arguments: &[String],
    user_index: usize,
    request_index: Option<usize>,
) -> Result<AuthenticatedIdentity, Box<dyn Error>> {
    let user_id = UserId::from_uuid(Uuid::parse_str(argument(arguments, user_index)?)?);
    let request_id = request_index
        .and_then(|index| arguments.get(index))
        .map(|value| Uuid::parse_str(value))
        .transpose()?
        .map_or_else(RequestId::new, RequestId::from_uuid);
    Ok(AuthenticatedIdentity::new(
        user_id,
        "hephaestus-operator",
        user_id.to_string(),
        json!({"interface": "operator_cli"}),
        request_id,
    ))
}

pub fn argument(arguments: &[String], index: usize) -> Result<&str, Box<dyn Error>> {
    arguments
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| usage().into())
}

pub const fn usage() -> &'static str {
    "usage: hephaestus-operator <inspect-release|inspect-instance|inspect-secret> \
       <actor-uuid> <object-uuid> [request-uuid]\n\
       hephaestus-operator metrics <actor-uuid> [request-uuid]\n\
       hephaestus-operator recover-update <actor-uuid> <update-uuid> \
       <retry|reject|resume> [request-uuid]\n\
       hephaestus-operator abandon-build <actor-uuid> <build-uuid> [request-uuid]\n\
       hephaestus-operator provision-image-catalog <manifest.json> [--dry-run]\n\
       hephaestus-operator registry-retention-report <inventory.json>"
}
