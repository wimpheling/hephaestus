use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, EffectClaim, ResourceAction, RetireExternalObservation,
};
use serde_json::Value;
use sqlx::PgPool;

use crate::{
    fixtures,
    harness::{self, Harness},
    support,
};

pub async fn claim(h: &Harness, external: bool) -> EffectClaim {
    let action = if external {
        ResourceAction::VerifyExternal
    } else {
        ResourceAction::Create
    };
    let request = h.effect(&h.identity, h.command, "data", action).await;
    h.repository
        .claim_resource_effect(&h.identity, request)
        .await
        .expect("actual committed claim")
}

pub async fn admit_cleanup(
    h: &Harness,
    identity: &AuthenticatedIdentity,
    target: &EffectClaim,
) -> RetireExternalObservation {
    let snapshot = h
        .repository
        .inspect(identity, h.intent.id())
        .await
        .expect("current authorized snapshot");
    let remove = fixtures::remove(identity, h.intent.id(), snapshot.version);
    h.repository
        .admit_remove(identity, remove)
        .await
        .expect("current admitted removal");
    retirement(h, identity, remove.command, target).await
}

pub async fn retirement(
    h: &Harness,
    identity: &AuthenticatedIdentity,
    command: recipe_application::CommandIdentity,
    target: &EffectClaim,
) -> RetireExternalObservation {
    let snapshot = h
        .repository
        .inspect(identity, h.intent.id())
        .await
        .expect("current CAS");
    RetireExternalObservation {
        command,
        target: target.clone(),
        provenance: AttemptProvenance::new(identity, harness::attempt())
            .expect("fresh retirement provenance"),
        expected_deployment_version: snapshot.version,
        expected_resource_version: snapshot.resources[&fixtures::key("data")].version,
    }
}

pub async fn lost_external_manager(pool: &PgPool, h: &Harness) -> AuthenticatedIdentity {
    sqlx::query("INSERT INTO project_maintainers (project_id,user_id) VALUES ($1,$2)")
        .bind(h.fixture.consuming_project)
        .bind(h.fixture.outsider.as_uuid())
        .execute(pool)
        .await
        .expect("target-only manager");
    sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
        .bind(h.fixture.maintainer.as_uuid())
        .execute(pool)
        .await
        .expect("original actor loses project/source authority");
    sqlx::query("UPDATE releases SET state='revoked', revoked_at=now() WHERE id=$1")
        .bind(h.fixture.release)
        .execute(pool)
        .await
        .expect("source withdrawn");
    sqlx::query("DELETE FROM agent_instance_state_volumes WHERE id=$1")
        .bind(h.fixture.volume)
        .execute(pool)
        .await
        .expect("external resource lost");
    support::identity(h.fixture.outsider)
}

pub async fn state(pool: &PgPool, target: &EffectClaim) -> Value {
    sqlx::query_scalar("SELECT recipe_effect_resource_state(resource) FROM recipe_deployment_resources resource WHERE deployment_id=$1 AND resource_name=$2")
        .bind(target.deployment_id.as_uuid()).bind(target.resource.as_str()).fetch_one(pool).await.expect("exact state")
}
