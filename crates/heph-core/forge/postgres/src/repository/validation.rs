use super::{helpers::storage, rows::RepositoryRow};
use forge_domain::RepositoryId;
use forge_service::{CreateRepository, ForgeRepositoryError};
use sqlx::{Postgres, Transaction};

pub fn validate_name(name: &str, message: &'static str) -> Result<(), ForgeRepositoryError> {
    if name.trim().is_empty() || name.len() > 200 {
        Err(ForgeRepositoryError::InvalidMetadata(message))
    } else {
        Ok(())
    }
}

pub fn validate_description(description: &str) -> Result<(), ForgeRepositoryError> {
    if description.chars().count() > 2_000 {
        Err(ForgeRepositoryError::InvalidMetadata(
            "project description must contain at most 2000 characters",
        ))
    } else {
        Ok(())
    }
}

pub fn validate_repository(input: &CreateRepository) -> Result<&str, ForgeRepositoryError> {
    validate_name(
        &input.name,
        "repository name must contain 1 to 200 characters",
    )?;
    input
        .default_branch
        .as_str()
        .strip_prefix("refs/heads/")
        .ok_or(ForgeRepositoryError::InvalidMetadata(
            "default branch must be beneath refs/heads/",
        ))
}

pub async fn insert_repository(
    transaction: &mut Transaction<'_, Postgres>,
    id: RepositoryId,
    input: &CreateRepository,
) -> Result<RepositoryRow, ForgeRepositoryError> {
    sqlx::query_as::<_, RepositoryRow>(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public, settings)
         VALUES ($1, $2, $3, $4, $5, jsonb_build_object('agent_runs_enabled', $6))
         RETURNING id, project_id, name, default_branch, is_public, settings, created_at",
    )
    .bind(id.as_uuid())
    .bind(input.project_id.as_uuid())
    .bind(&input.name)
    .bind(input.default_branch.as_str())
    .bind(input.is_public)
    .bind(input.agent_runs_enabled)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)
}
