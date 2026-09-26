use super::*;

#[derive(Clone, Copy)]
pub struct RunTarget {
    pub instance: AgentInstanceId,
    pub revision: AgentInstanceRevisionId,
    pub release: ReleaseId,
    pub release_agent: ReleaseAgentId,
    pub attachment: AgentAttachmentId,
}

#[derive(Clone, Copy)]
pub struct UpdateScenario {
    pub current: RunTarget,
    pub successful: RunTarget,
    pub rejected: RunTarget,
    pub uncertain: RunTarget,
}
