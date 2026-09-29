use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use uuid::Uuid;

/// Durable source facts captured immediately before the fork begins.
#[derive(Debug, Clone)]
pub struct SourceSessionState {
    pub repository_id: Uuid,
    pub session_id: Uuid,
    pub release_id: Uuid,
    pub release_agent_id: Uuid,
    pub model_rule_id: Uuid,
    pub instance_id: Uuid,
    pub revision_id: Uuid,
    pub attachment_id: Uuid,
    pub installation_id: Uuid,
    pub generation_id: Uuid,
    pub model_binding_id: Uuid,
    pub head: String,
    pub reachable_objects: BTreeSet<String>,
    pub record_blobs: BTreeMap<String, Vec<u8>>,
    pub accepted_receive_count: i64,
}

/// Target facts retained for the later fresh-instance/browser phase.
#[derive(Debug, Clone)]
pub struct ForkTargetState {
    pub source_repository_id: Uuid,
    pub source_head: String,
    pub source_accepted_receive_count: i64,
    pub source_release_id: Uuid,
    pub source_release_agent_id: Uuid,
    pub source_model_rule_id: Uuid,
    pub source_model_binding_id: Uuid,
    pub source_instance_id: Uuid,
    pub source_revision_id: Uuid,
    pub source_attachment_id: Uuid,
    pub source_installation_id: Uuid,
    pub source_generation_id: Uuid,
    pub repository_id: Uuid,
    pub checkout: PathBuf,
    pub session_id: Uuid,
    pub manifest_commit: String,
    pub manifest_path: String,
    pub initial_accepted_receive_count: i64,
}

/// Fresh target platform objects created after the fork is published.
#[allow(clippy::struct_field_names)] // Persisted platform IDs stay explicit at this test boundary.
#[derive(Debug, Clone, Copy)]
pub struct ForkTargetProvisioningState {
    pub instance_id: Uuid,
    pub revision_id: Uuid,
    pub attachment_id: Uuid,
    pub capability_revision_id: Uuid,
    pub bound_revision_id: Uuid,
    pub binding_id: Uuid,
    pub installation_id: Uuid,
    pub generation_id: Uuid,
}
