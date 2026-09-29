//! Stored PAT row decoding and scope conversion.

use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use identity_domain::{RequestId, UserId};
use pat_domain::{
    PersonalAccessTokenId, PersonalAccessTokenLabel, PersonalAccessTokenMetadata,
    PersonalAccessTokenRecord, PersonalAccessTokenScope, PersonalAccessTokenVerifier,
};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use super::super::model::PersonalAccessTokenServiceError;
use super::super::storage::storage;

#[derive(FromRow)]
pub(in crate::pat_postgres) struct PersonalAccessTokenRow {
    id: Uuid,
    verifier_version: i16,
    verifier_digest: Vec<u8>,
    owner_user_id: Uuid,
    label: String,
    git_operations: Vec<String>,
    repository_restrictions: Option<Vec<Uuid>>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
    creation_request_id: Uuid,
}

#[derive(FromRow)]
pub(in crate::pat_postgres) struct PersonalAccessTokenMetadataRow {
    id: Uuid,
    owner_user_id: Uuid,
    label: String,
    git_operations: Vec<String>,
    repository_restrictions: Option<Vec<Uuid>>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
    creation_request_id: Uuid,
}

impl PersonalAccessTokenRow {
    pub(in crate::pat_postgres) fn into_record(
        self,
    ) -> Result<PersonalAccessTokenRecord, PersonalAccessTokenServiceError> {
        let version = u16::try_from(self.verifier_version).map_err(storage)?;
        let digest = self
            .verifier_digest
            .try_into()
            .map_err(|_| PersonalAccessTokenServiceError::Persistence)?;
        let scope = parse_scope(self.git_operations, self.repository_restrictions)?;
        PersonalAccessTokenRecord::restore(
            PersonalAccessTokenId::from_uuid(self.id),
            PersonalAccessTokenVerifier::from_digest(version, digest),
            UserId::from_uuid(self.owner_user_id),
            PersonalAccessTokenLabel::parse(self.label)
                .map_err(|_| PersonalAccessTokenServiceError::Persistence)?,
            scope,
            self.created_at,
            self.expires_at,
            self.revoked_at,
            self.last_used_at,
            RequestId::from_uuid(self.creation_request_id),
        )
        .map_err(|_| PersonalAccessTokenServiceError::Persistence)
    }
}

impl PersonalAccessTokenMetadataRow {
    pub(in crate::pat_postgres) fn into_metadata(
        self,
    ) -> Result<PersonalAccessTokenMetadata, PersonalAccessTokenServiceError> {
        Ok(PersonalAccessTokenMetadata {
            id: PersonalAccessTokenId::from_uuid(self.id),
            owner_user_id: UserId::from_uuid(self.owner_user_id),
            label: PersonalAccessTokenLabel::parse(self.label)
                .map_err(|_| PersonalAccessTokenServiceError::Persistence)?,
            scope: parse_scope(self.git_operations, self.repository_restrictions)?,
            created_at: self.created_at,
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
            last_used_at: self.last_used_at,
            creation_request_id: RequestId::from_uuid(self.creation_request_id),
        })
    }
}

fn parse_operation(operation: &str) -> Result<GitOperation, PersonalAccessTokenServiceError> {
    match operation {
        "discover" => Ok(GitOperation::Discover),
        "fetch" => Ok(GitOperation::Fetch),
        "receive" => Ok(GitOperation::Receive),
        _ => Err(PersonalAccessTokenServiceError::Persistence),
    }
}

fn parse_scope(
    operations: Vec<String>,
    repository_restrictions: Option<Vec<Uuid>>,
) -> Result<PersonalAccessTokenScope, PersonalAccessTokenServiceError> {
    let operations = operations
        .into_iter()
        .map(|operation| parse_operation(&operation))
        .collect::<Result<Vec<_>, _>>()?;
    let restrictions = repository_restrictions.map(|repositories| {
        repositories
            .into_iter()
            .map(RepositoryId::from_uuid)
            .collect::<Vec<_>>()
    });
    PersonalAccessTokenScope::new(operations, restrictions)
        .map_err(|_| PersonalAccessTokenServiceError::Persistence)
}
