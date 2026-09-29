use super::super::SeededInstance;
use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use uuid::Uuid;

pub struct RetirementContext<'a> {
    pub pool: &'a PgPool,
    pub running: &'a hephaestus_app::RunningHephaestus,
    pub token_factory: &'a (dyn Fn(&str) -> String + Send + Sync),
    pub owner: &'a AuthenticatedIdentity,
    pub outsider: &'a AuthenticatedIdentity,
    pub instance: &'a SeededInstance,
    pub retained_run_id: Uuid,
    pub retry_instance: &'a SeededInstance,
    pub retry_source_run_id: Uuid,
    pub retry_repository_id: Uuid,
    pub gateway_id: Uuid,
    pub project_id: Uuid,
    pub mailbox_id: Uuid,
    pub public_url: &'a str,
    pub valid_inbound_credential: &'a str,
    pub import_parameters: Vec<rpc_proto::messages::hephaestus::common::v1::ParameterValue>,
}

pub(crate) struct ProvenanceBaseline {
    pub(crate) snapshot_id: String,
    pub(crate) model_version: String,
    pub(crate) snapshot_hash: String,
    pub(crate) use_ids: Vec<String>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct RetryObservation {
    pub(crate) id: Uuid,
    pub(crate) state: String,
    pub(crate) outcome: Option<String>,
    pub(crate) failure: Option<String>,
    pub(crate) vm_id: Option<String>,
}
