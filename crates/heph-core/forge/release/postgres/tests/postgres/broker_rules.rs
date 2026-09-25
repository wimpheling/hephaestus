//! Explicit broker-rule copy integration scenario.

use super::*;
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use release_postgres::BrokeredRuleCopy;
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport, RotateSecret,
};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};

#[path = "broker_rules/fixtures.rs"]
mod fixtures;
#[path = "broker_rules/updates.rs"]
mod updates;

struct PreparedBrokerRules {
    actor: AuthenticatedIdentity,
    instance_id: AgentInstanceId,
    revision: AgentInstanceRevisionId,
    source_model_rule_id: Uuid,
    source_relay_rule_id: Uuid,
    source_rules: Vec<(Uuid, Uuid)>,
    source_rules_before: Vec<BrokerRuleSnapshot>,
    active_versions: BTreeMap<Uuid, Uuid>,
    candidate_agent_id: ReleaseAgentId,
}

async fn prepare(pool: &PgPool, service: &ReleaseService) -> PreparedBrokerRules {
    let fixture = seed_with_config(pool, &brokered_config()).await;
    fixtures::seed_secret_manager_roles(pool, &fixture).await;
    let actor = identity(fixture.actor);
    let (
        release_id,
        release_agent_id,
        instance_id,
        initial_revision,
        source_model_rule_id,
        source_relay_rule_id,
        attachment_id,
    ) = fixtures::publish_broker_instance(service, &fixture, &actor).await;
    let secret_service = fixtures::new_secret_service(pool);
    let (revision, imports) = fixtures::create_broker_secrets(
        &secret_service,
        &fixture,
        &actor,
        instance_id,
        initial_revision,
        attachment_id,
    )
    .await;
    let source_rules = fixtures::declare_source_rules(
        pool,
        &secret_service,
        &actor,
        &imports,
        revision,
        source_model_rule_id,
        source_relay_rule_id,
    )
    .await;
    let (source_rules_before, active_versions) =
        fixtures::rotate_source_secrets(pool, &secret_service, &actor, &imports, revision).await;
    let candidate_agent_id = seed_matching_update_release(pool, release_id, release_agent_id).await;
    PreparedBrokerRules {
        actor,
        instance_id,
        revision,
        source_model_rule_id,
        source_relay_rule_id,
        source_rules,
        source_rules_before,
        active_versions,
        candidate_agent_id,
    }
}

#[tokio::test]
#[serial]
async fn explicit_broker_rule_copies_clone_active_rules_atomically() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let scenario = prepare(&pool, &service).await;
    updates::assert_updates(&pool, &service, &scenario).await;
}
