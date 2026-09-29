//! instance lookup scenario.

use super::support::{seed_gateway_with_instance_state, test_pool, worker_pool};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceInstancePage, GatewayServiceInstanceState,
    GatewayServiceOwner, GatewayServiceOwnership, GatewayServiceTargetPage,
    GatewayServiceTargetStore, MAX_SERVICE_INSTANCE_PAGE_SIZE, MAX_SERVICE_TARGET_PAGE_SIZE,
};
use gateway_postgres::{PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets};
use serial_test::serial;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exact_service_instance_lookup_survives_fencing_and_cleanup() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceTargets::new(worker.clone());
    let fixture = seed_gateway_with_instance_state(
        &pool,
        "exact-instance",
        "enabled",
        "published",
        "starting",
        true,
        None,
    )
    .await;
    let identity = GatewayServiceIdentity {
        instance_id: fixture.old_instance,
        gateway_id: fixture.gateway,
        revision_id: fixture.old_service,
    };

    let observed = store
        .get_service_instance(identity)
        .await
        .expect("lookup starting instance")
        .expect("starting instance exists");
    assert_eq!(observed.identity, identity);
    assert_eq!(observed.state, GatewayServiceInstanceState::Starting);
    assert_eq!(observed.fencing_token, 1);

    sqlx::query("UPDATE gateway_service_instances SET state = 'stopping' WHERE id = $1")
        .bind(fixture.old_instance)
        .execute(&pool)
        .await
        .expect("transition instance to stopping");
    let observed = store
        .get_service_instance(identity)
        .await
        .expect("lookup stopping instance")
        .expect("stopping instance exists");
    assert_eq!(observed.state, GatewayServiceInstanceState::Stopping);

    let recovery_owner =
        GatewayServiceOwner::new(&fixture.owner_host_id, Uuid::new_v4()).expect("owner");
    let ownership = PostgresGatewayServiceOwnership::new(worker);
    let recovered = ownership
        .claim_expired(&recovery_owner, Duration::from_secs(30), 1)
        .await
        .expect("claim expired instance");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].identity, identity);
    assert_eq!(recovered[0].fencing_token, 2);
    assert_eq!(recovered[0].owner_uuid, recovery_owner.owner_uuid);
    assert_eq!(recovered[0].state, GatewayServiceInstanceState::Stopping);

    let observed = store
        .get_service_instance(identity)
        .await
        .expect("lookup recovered instance")
        .expect("recovered instance exists");
    assert_eq!(observed.fencing_token, 2);
    assert_eq!(observed.owner_uuid, recovery_owner.owner_uuid);
    assert_eq!(observed.state, GatewayServiceInstanceState::Stopping);

    ownership
        .mark_cleaned(&recovered[0], &recovery_owner)
        .await
        .expect("mark instance cleaned");
    let cleaned = store
        .get_service_instance(identity)
        .await
        .expect("lookup cleaned instance")
        .expect("cleaned instance remains queryable");
    assert_eq!(cleaned.state, GatewayServiceInstanceState::Cleaned);
    assert_eq!(cleaned.identity, identity);

    assert!(
        store
            .get_service_instance(GatewayServiceIdentity {
                gateway_id: Uuid::new_v4(),
                ..identity
            })
            .await
            .expect("wrong gateway lookup")
            .is_none()
    );
    assert!(
        store
            .get_service_instance(GatewayServiceIdentity {
                revision_id: Uuid::new_v4(),
                ..identity
            })
            .await
            .expect("wrong revision lookup")
            .is_none()
    );
    assert!(
        store
            .get_service_instance(GatewayServiceIdentity {
                instance_id: Uuid::new_v4(),
                ..identity
            })
            .await
            .expect("unknown instance lookup")
            .is_none()
    );
    for invalid_identity in [
        GatewayServiceIdentity {
            instance_id: Uuid::nil(),
            ..identity
        },
        GatewayServiceIdentity {
            gateway_id: Uuid::nil(),
            ..identity
        },
        GatewayServiceIdentity {
            revision_id: Uuid::nil(),
            ..identity
        },
    ] {
        assert!(store.get_service_instance(invalid_identity).await.is_err());
    }
}

#[test]
fn service_target_page_rejects_unbounded_or_nil_cursors() {
    assert!(GatewayServiceTargetPage::new(None, 1).is_ok());
    assert!(GatewayServiceTargetPage::new(None, MAX_SERVICE_TARGET_PAGE_SIZE).is_ok());
    assert!(GatewayServiceTargetPage::new(None, 0).is_err());
    assert!(GatewayServiceTargetPage::new(None, MAX_SERVICE_TARGET_PAGE_SIZE + 1).is_err());
    assert!(GatewayServiceTargetPage::new(Some(Uuid::nil()), 1).is_err());
    assert!(GatewayServiceInstancePage::new("valid-host", None, 1).is_ok());
    assert!(GatewayServiceInstancePage::new("", None, 1).is_err());
    assert!(GatewayServiceInstancePage::new("invalid host", None, 1).is_err());
    assert!(GatewayServiceInstancePage::new("valid-host", None, 0).is_err());
    assert!(GatewayServiceInstancePage::new("valid-host", Some(Uuid::nil()), 1).is_err());
    assert!(
        GatewayServiceInstancePage::new("valid-host", None, MAX_SERVICE_INSTANCE_PAGE_SIZE + 1)
            .is_err()
    );
}
