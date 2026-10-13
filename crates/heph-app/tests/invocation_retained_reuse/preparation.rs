use crate::fixtures;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId,
    DeploymentExecutionPreparationRepository, DeploymentId, DeploymentIntent, EffectClaim,
    PreparedInstanceImport, ResourceAction, ResourceEffect,
};
use std::collections::BTreeMap;
use uuid::Uuid;

pub struct Reuse {
    pub identity: identity_domain::AuthenticatedIdentity,
    pub claim: EffectClaim,
    pub command: CommandIdentity,
    pub intent: DeploymentIntent,
    pub input: PreparedInstanceImport,
}
pub async fn external_claim(
    f: &crate::retained_fixture::Fixture,
    user: identity_domain::UserId,
) -> Reuse {
    let identity = crate::support::identity(user);
    let mut seeded = f.birth.h.fixture;
    fresh_source(f, &mut seeded).await;
    seeded.volume = f
        .ready
        .claim
        .purpose
        .receipt()
        .intent
        .registration()
        .id()
        .as_uuid();
    // Immutable recipe definition identity cannot be reused for owned->external shape.
    let source = fixtures::source(&seeded, true)
        .replace("recipe_id = \"database\"", "recipe_id = \"database-reuse\"");
    let intent = fixtures::build(
        &seeded,
        &source,
        DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
        "explicit-ready-reuse",
        &BTreeMap::new(),
        &[fixtures::catalog(&seeded)],
        true,
    );
    let install = fixtures::install(&identity, intent.clone());
    let command = install.command;
    f.birth
        .h
        .repository
        .admit_install(&identity, install)
        .await
        .unwrap();
    let prepared = f
        .birth
        .h
        .preparation
        .prepare_execution(&identity, command, intent.id())
        .await
        .unwrap();
    let snapshot = f
        .birth
        .h
        .repository
        .inspect(&identity, intent.id())
        .await
        .unwrap();
    let claim = f
        .birth
        .h
        .repository
        .claim_resource_effect(
            &identity,
            ResourceEffect {
                command,
                deployment_id: intent.id(),
                resource: fixtures::key("data"),
                expected_deployment_version: snapshot.version,
                expected_resource_version: snapshot.resources[&fixtures::key("data")].version,
                action: ResourceAction::VerifyExternal,
                provenance: AttemptProvenance::new(
                    &identity,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    Reuse {
        identity,
        claim,
        command,
        intent,
        input: prepared.preparation.instances()[0].clone(),
    }
}

async fn fresh_source(f: &crate::retained_fixture::Fixture, seeded: &mut crate::support::Fixture) {
    let release = Uuid::new_v4();
    let agent = Uuid::new_v4();
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state) SELECT $1,repository_id,'retained-reuse-v1',source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,'draft' FROM releases WHERE id=$2")
        .bind(release).bind(seeded.release).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state,parameter_schema) SELECT $1,$2,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state,parameter_schema FROM release_agents WHERE id=$3")
        .bind(agent).bind(release).bind(seeded.release_agent).execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE releases SET state='published',published_at=now() WHERE id=$1")
        .bind(release)
        .execute(&f.pool)
        .await
        .unwrap();
    seeded.release = release;
    seeded.release_agent = agent;
}
