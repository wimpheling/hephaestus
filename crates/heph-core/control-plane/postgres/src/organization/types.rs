use time::OffsetDateTime;
use uuid::Uuid;

/// Stable organization summary returned by list queries.
pub struct OrganizationSummary {
    pub id: Uuid,
    pub name: String,
    pub project_count: i64,
    pub repository_count: i64,
}

/// Organization details visible to the current actor.
pub struct Organization {
    pub id: Uuid,
    pub name: String,
}

/// Repository summary visible within an organization.
pub struct RepositorySummary {
    pub id: Uuid,
    pub name: String,
    pub default_branch: String,
    pub is_public: bool,
    pub project_name: String,
    pub run_count: i64,
    pub last_run_at: Option<OffsetDateTime>,
}

/// Project summary visible within an organization.
pub struct ProjectSummary {
    pub id: Uuid,
    pub name: String,
    pub repository_count: i64,
    pub instance_count: i64,
    pub run_count: i64,
    pub last_activity_at: Option<OffsetDateTime>,
}

/// Stable cursor page for organization summaries.
pub struct OrganizationPage {
    pub size: i64,
    pub after: Option<Uuid>,
}

/// Result of an organization page query.
pub struct OrganizationPageResult {
    pub organizations: Vec<OrganizationSummary>,
    pub next_page_token: Option<String>,
}

/// Result of an organization repository page query.
pub struct RepositoryPageResult {
    pub repositories: Vec<RepositorySummary>,
    pub next_page_token: Option<String>,
}

/// Result of an organization project page query.
pub struct ProjectPageResult {
    pub projects: Vec<ProjectSummary>,
    pub next_page_token: Option<String>,
}

/// Typed organization application failure.
#[derive(Debug, thiserror::Error)]
pub enum OrganizationError {
    /// Persistence failed while evaluating the authorized query.
    #[error("organization query failed")]
    Persistence(#[source] sqlx::Error),
    /// A bounded page size could not be represented on this platform.
    #[error("organization page is invalid")]
    InvalidPage,
    /// The requested organization is absent or not visible to the actor.
    #[error("organization was not found")]
    NotFound,
}
