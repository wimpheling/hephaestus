use super::*;

/// Value-free secret metadata query failure.
#[derive(Debug, thiserror::Error)]
pub enum SecretQueryError {
    /// Query is unavailable in the current adapter.
    #[error("secret metadata query unavailable")]
    Unavailable,
    /// Invalid cursor page.
    #[error("invalid secret page")]
    InvalidPage,
    /// Database query failure.
    #[error("secret metadata persistence failed")]
    Persistence(#[source] sqlx::Error),
}
/// Cursor page request.
#[derive(Clone, Copy)]
pub struct Page {
    pub size: i64,
    pub after: Option<Uuid>,
}
/// Cursor page result.
pub struct PageResult<T> {
    pub values: Vec<T>,
    pub next_page_token: Option<String>,
}
/// Value-free secret summary.
pub struct SecretSummary {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub allowed_delivery_modes: Vec<String>,
    pub active_version_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub active_version_sequence: Option<i64>,
    pub active_version_created_at: Option<OffsetDateTime>,
    pub grant_count: i64,
    pub import_count: i64,
    pub binding_count: i64,
    pub has_raw_binding: bool,
    pub can_rotate: bool,
    pub can_manage_grants: bool,
    pub can_revoke: bool,
    pub can_purge: bool,
}
/// Value-free grant summary.
pub struct GrantSummary {
    pub id: Uuid,
    pub secret_id: Uuid,
    pub secret_name: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub target_name: Option<String>,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub import_count: i64,
    pub import_id: Option<Uuid>,
    pub import_alias: Option<String>,
    pub import_status: Option<String>,
}
/// Value-free import summary.
pub struct ImportSummary {
    pub id: Uuid,
    pub alias: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub status: String,
    pub secret_id: Uuid,
    pub secret_name: String,
    pub secret_status: String,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
    pub active_version_id: Option<Uuid>,
}
#[derive(FromRow)]
pub struct SecretRow {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub allowed_delivery_modes: Vec<String>,
    pub active_version_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub active_version_sequence: Option<i64>,
    pub active_version_created_at: Option<OffsetDateTime>,
    pub grant_count: i64,
    pub import_count: i64,
    pub binding_count: i64,
    pub has_raw_binding: bool,
    pub can_rotate: bool,
    pub can_manage_grants: bool,
    pub can_revoke: bool,
    pub can_purge: bool,
}

#[derive(FromRow)]
pub struct GrantRow {
    pub id: Uuid,
    pub secret_id: Uuid,
    pub secret_name: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub target_name: Option<String>,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub import_count: i64,
    pub import_id: Option<Uuid>,
    pub import_alias: Option<String>,
    pub import_status: Option<String>,
}

#[derive(FromRow)]
pub struct ImportRow {
    pub id: Uuid,
    pub alias: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub status: String,
    pub secret_id: Uuid,
    pub secret_name: String,
    pub secret_status: String,
    pub active_version_id: Option<Uuid>,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
}

pub fn validate_page(page: Page) -> Result<(), SecretQueryError> {
    if (1..=100).contains(&page.size) {
        Ok(())
    } else {
        Err(SecretQueryError::InvalidPage)
    }
}

pub fn finish_secret_page(
    mut rows: Vec<SecretRow>,
    page: Page,
) -> Result<PageResult<SecretSummary>, SecretQueryError> {
    let take = usize::try_from(page.size).map_err(|_| SecretQueryError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next_page_token = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| row.id.to_string());
    Ok(PageResult {
        values: rows.into_iter().map(SecretSummary::from).collect(),
        next_page_token,
    })
}

pub fn finish_grant_page(
    mut rows: Vec<GrantRow>,
    page: Page,
) -> Result<PageResult<GrantSummary>, SecretQueryError> {
    let take = usize::try_from(page.size).map_err(|_| SecretQueryError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next_page_token = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| row.id.to_string());
    Ok(PageResult {
        values: rows.into_iter().map(GrantSummary::from).collect(),
        next_page_token,
    })
}

pub fn finish_import_page(
    mut rows: Vec<ImportRow>,
    page: Page,
) -> Result<PageResult<ImportSummary>, SecretQueryError> {
    let take = usize::try_from(page.size).map_err(|_| SecretQueryError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next_page_token = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| row.id.to_string());
    Ok(PageResult {
        values: rows.into_iter().map(ImportSummary::from).collect(),
        next_page_token,
    })
}

impl From<SecretRow> for SecretSummary {
    fn from(row: SecretRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            status: row.status,
            allowed_delivery_modes: row.allowed_delivery_modes,
            active_version_id: row.active_version_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            active_version_sequence: row.active_version_sequence,
            active_version_created_at: row.active_version_created_at,
            grant_count: row.grant_count,
            import_count: row.import_count,
            binding_count: row.binding_count,
            has_raw_binding: row.has_raw_binding,
            can_rotate: row.can_rotate,
            can_manage_grants: row.can_manage_grants,
            can_revoke: row.can_revoke,
            can_purge: row.can_purge,
        }
    }
}

impl From<GrantRow> for GrantSummary {
    fn from(row: GrantRow) -> Self {
        Self {
            id: row.id,
            secret_id: row.secret_id,
            secret_name: row.secret_name,
            target_kind: row.target_kind,
            target_id: row.target_id,
            target_name: row.target_name,
            delivery_modes: row.delivery_modes,
            phases: row.phases,
            destinations: row.destinations,
            expires_at: row.expires_at,
            status: row.status,
            created_at: row.created_at,
            import_count: row.import_count,
            import_id: row.import_id,
            import_alias: row.import_alias,
            import_status: row.import_status,
        }
    }
}

impl From<ImportRow> for ImportSummary {
    fn from(row: ImportRow) -> Self {
        Self {
            id: row.id,
            alias: row.alias,
            target_kind: row.target_kind,
            target_id: row.target_id,
            status: row.status,
            secret_id: row.secret_id,
            secret_name: row.secret_name,
            secret_status: row.secret_status,
            delivery_modes: row.delivery_modes,
            phases: row.phases,
            destinations: row.destinations,
            expires_at: row.expires_at,
            active_version_id: row.active_version_id,
        }
    }
}
/// Project secret authority pages.
pub struct ProjectAuthority {
    pub grants: PageResult<GrantSummary>,
    pub imports: PageResult<ImportSummary>,
}
