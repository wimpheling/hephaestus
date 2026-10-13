use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    BeginOwnedProvisioning, OwnedProvisioningContext, OwnedProvisioningDiscovery,
    OwnedProvisioningExpectation, VolumeOwnedProvisioningRepository,
    VolumeOwnedRegistrationRepository, VolumeRootNamespaceId,
};

pub use crate::provisioning::revoke;
pub use crate::seals::{Fixture, identity, intent};

pub async fn setup(
    pool: &PgPool,
) -> (
    Fixture,
    OwnedProvisioningExpectation,
    BeginOwnedProvisioning,
) {
    crate::private_migrations::checkpoint(111)
        .run(pool)
        .await
        .unwrap();
    let fixture = crate::seals::seed(pool).await;
    let registration = intent(&fixture);
    let expected = OwnedProvisioningExpectation::new(
        registration.clone(),
        "owned-host".into(),
        "/owned-volumes".into(),
        VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
    )
    .unwrap();
    let receipt = fixture
        .actor
        .register_owned(&identity(fixture.owner), &registration)
        .await
        .unwrap();
    let request = BeginOwnedProvisioning {
        purpose: expected.purpose(receipt).unwrap(),
        operation_id: expected.first_operation_id(),
        expected_generation: 0,
    };
    (fixture, expected, request)
}
pub async fn begin(
    fixture: &Fixture,
    request: &BeginOwnedProvisioning,
) -> OwnedProvisioningContext {
    fixture
        .actor
        .begin_owned_provisioning(&identity(fixture.owner), request)
        .await
        .unwrap()
}
pub async fn assert_original(
    fixture: &Fixture,
    expected: &OwnedProvisioningExpectation,
    original: &OwnedProvisioningContext,
) {
    assert_eq!(
        fixture
            .worker
            .discover_owned_provisioning(expected)
            .await
            .unwrap(),
        OwnedProvisioningDiscovery::Original(Box::new(original.clone()))
    );
}
/// Observe exact tables to prove discovery did not allocate operations or events.
pub async fn snapshot(pool: &PgPool, expected: &OwnedProvisioningExpectation) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('volume',(SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=$1),'purposes',(SELECT COALESCE(jsonb_agg(to_jsonb(purpose) ORDER BY volume_id),'[]'::jsonb) FROM volume_owned_backing_purposes purpose WHERE volume_id=$1),'operations',(SELECT COALESCE(jsonb_agg(to_jsonb(operation) ORDER BY generation),'[]'::jsonb) FROM volume_owned_provisioning_operations operation WHERE volume_id=$1),'observations',(SELECT COALESCE(jsonb_agg(to_jsonb(observation) ORDER BY id),'[]'::jsonb) FROM volume_owned_backing_observations observation WHERE volume_id=$1),'proofs',(SELECT COALESCE(jsonb_agg(to_jsonb(proof) ORDER BY id),'[]'::jsonb) FROM volume_owned_transition_proofs proof WHERE volume_id=$1),'events',(SELECT count(*) FROM application_events),'audits',(SELECT count(*) FROM authorization_audit_events))")
        .bind(expected.intent().registration().id().as_uuid()).fetch_one(pool).await.unwrap()
}
