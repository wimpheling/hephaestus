//! Worker-owned bounded service-log retention and maintenance contracts.

use async_trait::async_trait;
use uuid::Uuid;

/// Maximum chunks one bounded retention transaction may inspect.
pub const MAX_SERVICE_LOG_MAINTENANCE_CHUNKS: usize = 256;
/// Maximum epoch metadata rows one bounded retention transaction may inspect.
pub const MAX_SERVICE_LOG_MAINTENANCE_EPOCHS: usize = 32;
/// Maximum retained fencing epochs represented in one project's log metadata.
pub const MAX_SERVICE_LOG_PROJECT_EPOCHS: u32 = 128;

/// Bounded work policy for one worker-owned retention transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenancePolicy {
    /// Maximum payload chunks deleted in one transaction. TTL and compensated
    /// pressure scans may inspect more candidates, but deletion is bounded.
    pub max_chunks: usize,
    /// Maximum empty epoch rows deleted in one transaction. Flag updates may
    /// lock the bounded project epoch set in addition to these deletions.
    pub max_epochs: usize,
}

impl GatewayServiceLogMaintenancePolicy {
    /// Creates a bounded retention policy.
    #[must_use]
    pub const fn new(max_chunks: usize, max_epochs: usize) -> Self {
        Self {
            max_chunks,
            max_epochs,
        }
    }

    /// Validates the policy against platform work limits.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.max_chunks > 0
            && self.max_chunks <= MAX_SERVICE_LOG_MAINTENANCE_CHUNKS
            && self.max_epochs > 0
            && self.max_epochs <= MAX_SERVICE_LOG_MAINTENANCE_EPOCHS
    }
}

impl Default for GatewayServiceLogMaintenancePolicy {
    fn default() -> Self {
        Self::new(
            MAX_SERVICE_LOG_MAINTENANCE_CHUNKS,
            MAX_SERVICE_LOG_MAINTENANCE_EPOCHS,
        )
    }
}

/// Redacted result of one bounded retention transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenanceReport {
    /// Chunks removed because their server retention time elapsed.
    pub expired_chunks: usize,
    /// Bytes removed because their server retention time elapsed.
    pub expired_bytes: usize,
    /// Chunks removed under instance or project pressure.
    pub evicted_chunks: usize,
    /// Bytes removed under instance or project pressure.
    pub evicted_bytes: usize,
    /// Empty, permanently ineligible epoch metadata rows removed.
    pub metadata_epochs: usize,
    /// Whether another bounded transaction may have eligible work.
    pub has_more: bool,
}

/// Safe failures for a worker-owned retention transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceLogMaintenanceError {
    /// The project or bounded policy is malformed.
    #[error("invalid gateway service log maintenance argument")]
    InvalidArgument,
    /// `PostgreSQL` could not complete the bounded transaction.
    #[error("gateway service log maintenance is unavailable")]
    Unavailable,
}

/// Maximum number of projects returned by one maintenance enumeration page.
pub const MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE: u16 = 128;

/// Bounded keyset page for worker-owned service-log maintenance projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenanceProjectPage {
    /// Return projects strictly after this stable project identity.
    pub after: Option<Uuid>,
    /// Maximum number of project identities to return.
    pub limit: u16,
}

impl GatewayServiceLogMaintenanceProjectPage {
    /// Creates a validated maintenance-project page.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceLogMaintenanceError::InvalidArgument`] for a
    /// zero or oversized page, or a nil cursor.
    pub fn new(after: Option<Uuid>, limit: u16) -> Result<Self, GatewayServiceLogMaintenanceError> {
        let page = Self { after, limit };
        page.validate()?;
        Ok(page)
    }

    /// Validates a page assembled by an internal scheduler.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceLogMaintenanceError::InvalidArgument`] when
    /// the page limit or cursor is outside the bounded contract.
    pub fn validate(&self) -> Result<(), GatewayServiceLogMaintenanceError> {
        if self.limit == 0
            || self.limit > MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE
            || self.after.is_some_and(|value| value.is_nil())
        {
            return Err(GatewayServiceLogMaintenanceError::InvalidArgument);
        }
        Ok(())
    }
}

/// Bounded UUID page returned by the worker-only maintenance index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenanceProjectPageResult {
    /// Project identities backed by durable log usage rows.
    pub projects: Vec<Uuid>,
    /// Cursor for the next keyset page, if one exists.
    pub next_after: Option<Uuid>,
}

/// Worker-only enumeration of projects that have durable service-log state.
#[async_trait]
pub trait GatewayServiceLogMaintenanceProjects: Send + Sync {
    /// Lists project usage rows in stable UUID order, including projects with
    /// no active service instance. A scheduler must keep failed projects
    /// eligible for a later bounded retry while advancing fairly, so one
    /// transient failure does not block the rest of a sweep.
    async fn list_projects(
        &self,
        page: GatewayServiceLogMaintenanceProjectPage,
    ) -> Result<GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceError>;
}

/// Worker-only bounded retention and metadata maintenance port.
#[async_trait]
pub trait GatewayServiceLogMaintenance: Send + Sync {
    /// Performs one bounded server-clock TTL, pressure, and metadata pass.
    async fn maintain_project(
        &self,
        project_id: Uuid,
        policy: GatewayServiceLogMaintenancePolicy,
    ) -> Result<GatewayServiceLogMaintenanceReport, GatewayServiceLogMaintenanceError>;
}
