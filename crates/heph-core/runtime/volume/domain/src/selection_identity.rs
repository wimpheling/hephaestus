use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId, RunId};
use serde::Serialize;
use uuid::Uuid;

use crate::{VolumeContractError, VolumeMountScope};

/// Exact immutable consumer selected by a run, including stable project ownership.
///
/// This value carries no authority. The repository must verify its fields against
/// the run's stored revision, rather than substituting the current active revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RunVolumeIdentity {
    run: RunId,
    instance: AgentInstanceId,
    revision: AgentInstanceRevisionId,
    release: ReleaseId,
    release_agent: ReleaseAgentId,
    project: Uuid,
}

impl RunVolumeIdentity {
    /// Constructs a non-nil exact identity for acquisition or inspection.
    ///
    /// # Errors
    ///
    /// Rejects nil identifiers. Storage remains responsible for authoritative equality.
    pub fn new(
        run_id: RunId,
        instance_id: AgentInstanceId,
        revision_id: AgentInstanceRevisionId,
        release_id: ReleaseId,
        release_agent_id: ReleaseAgentId,
        project_id: Uuid,
    ) -> Result<Self, VolumeContractError> {
        if [
            run_id.as_uuid(),
            instance_id.as_uuid(),
            revision_id.as_uuid(),
            release_id.as_uuid(),
            release_agent_id.as_uuid(),
            project_id,
        ]
        .iter()
        .any(Uuid::is_nil)
        {
            return Err(VolumeContractError::InvalidRunSelection);
        }
        Ok(Self {
            run: run_id,
            instance: instance_id,
            revision: revision_id,
            release: release_id,
            release_agent: release_agent_id,
            project: project_id,
        })
    }

    /// Durable run identifier.
    #[must_use]
    pub const fn run_id(self) -> RunId {
        self.run
    }
    /// Exact consumer instance.
    #[must_use]
    pub const fn instance_id(self) -> AgentInstanceId {
        self.instance
    }
    /// Exact immutable consumer revision.
    #[must_use]
    pub const fn revision_id(self) -> AgentInstanceRevisionId {
        self.revision
    }
    /// Exact released software identity.
    #[must_use]
    pub const fn release_id(self) -> ReleaseId {
        self.release
    }
    /// Exact released export identity.
    #[must_use]
    pub const fn release_agent_id(self) -> ReleaseAgentId {
        self.release_agent
    }
    /// Stable resource and consumer owning project.
    #[must_use]
    pub const fn project_id(self) -> Uuid {
        self.project
    }

    pub(crate) fn matches_scope(self, scope: &VolumeMountScope) -> bool {
        self.instance == scope.instance_id()
            && self.revision == scope.revision_id()
            && self.release_agent == scope.release_agent_id()
    }
}
