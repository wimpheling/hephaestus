//! target boundaries scenario.

use super::support::{
    insert_accepted_invocation, seed_gateway, set_pointers, test_pool, worker_pool,
};
use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceInstanceKey, GatewayServiceTargetPage,
    GatewayServiceTargetStore, MAX_SERVICE_TARGET_PAGE_SIZE,
};
use gateway_postgres::PostgresGatewayServiceTargets;
use serial_test::serial;
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_targets_preserve_serving_candidate_and_lifecycle_boundaries() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceTargets::new(worker);
    let serving_and_revoked = seed_gateway(&pool, "revoked-candidate", "enabled", "revoked").await;
    set_pointers(
        &pool,
        serving_and_revoked.gateway,
        serving_and_revoked.old_service,
        Some(serving_and_revoked.candidate_service),
    )
    .await;
    insert_accepted_invocation(&pool, &serving_and_revoked).await;

    let mixed = seed_gateway(&pool, "mixed-stateless-service", "enabled", "published").await;
    set_pointers(
        &pool,
        mixed.gateway,
        mixed.stateless,
        Some(mixed.candidate_service),
    )
    .await;
    insert_accepted_invocation(&pool, &mixed).await;

    let paused = seed_gateway(&pool, "paused-service", "paused", "published").await;
    set_pointers(
        &pool,
        paused.gateway,
        paused.old_service,
        Some(paused.candidate_service),
    )
    .await;

    let stateless_only = seed_gateway(&pool, "stateless-only", "enabled", "published").await;
    set_pointers(
        &pool,
        stateless_only.gateway,
        stateless_only.stateless,
        None,
    )
    .await;

    let listed = tokio::time::timeout(Duration::from_secs(30), async {
        let mut cursor = None;
        let mut listed = Vec::new();
        loop {
            let page = GatewayServiceTargetPage::new(cursor, 1).expect("bounded page");
            assert!(page.limit <= MAX_SERVICE_TARGET_PAGE_SIZE);
            let result = store
                .list_service_targets(page)
                .await
                .expect("list service targets");
            assert!(result.targets.len() <= 1);
            listed.extend(result.targets);
            let Some(next) = result.next_after else {
                break;
            };
            assert_ne!(Some(next), cursor);
            if let Some(previous) = cursor {
                assert!(next > previous);
            }
            cursor = Some(next);
        }
        listed
    })
    .await
    .expect("service target pagination completes");
    let listed_ids: HashSet<_> = listed.iter().map(|target| target.gateway_id).collect();
    assert!(listed_ids.contains(&serving_and_revoked.gateway));
    assert!(listed_ids.contains(&mixed.gateway));
    assert!(!listed_ids.contains(&paused.gateway));
    assert!(!listed_ids.contains(&stateless_only.gateway));

    let serving = listed
        .iter()
        .find(|target| target.gateway_id == serving_and_revoked.gateway)
        .expect("serving and candidate target");
    let active = serving
        .active_service_revision
        .as_ref()
        .expect("published serving service");
    assert_eq!(active.revision_id, serving_and_revoked.old_service);
    assert_eq!(
        active.service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );
    assert_eq!(active.release_state.as_deref(), Some("published"));
    assert!(active.publication_eligible);
    let candidate = serving
        .desired_service_revision
        .as_ref()
        .expect("revoked desired service");
    assert_eq!(candidate.revision_id, serving_and_revoked.candidate_service);
    assert_eq!(
        candidate.service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );
    assert_eq!(candidate.release_state.as_deref(), Some("revoked"));
    assert!(!candidate.publication_eligible);

    let mixed_target = listed
        .iter()
        .find(|target| target.gateway_id == mixed.gateway)
        .expect("mixed target");
    assert!(mixed_target.active_service_revision.is_none());
    assert_eq!(
        mixed_target
            .desired_service_revision
            .as_ref()
            .expect("mixed desired service")
            .revision_id,
        mixed.candidate_service
    );
    assert_eq!(
        mixed_target
            .desired_service_revision
            .as_ref()
            .expect("mixed desired service")
            .service
            .log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );

    let paused_target = store
        .get_service_target(paused.gateway, paused.old_service)
        .await
        .expect("exact paused target query")
        .expect("paused target exists");
    assert_eq!(paused_target.lifecycle, "paused");
    assert_eq!(paused_target.revision.revision_id, paused.old_service);
    assert_eq!(
        paused_target.revision.service.log_capture_mode,
        gateway_domain::ServiceLogCaptureMode::Application
    );
    assert_eq!(
        paused_target.desired_service_revision_id,
        Some(paused.candidate_service)
    );
    assert!(
        store
            .get_service_target(serving_and_revoked.gateway, mixed.candidate_service)
            .await
            .expect("cross-revision target query")
            .is_none()
    );

    assert_eq!(
        store
            .count_accepted_service_invocations(
                serving_and_revoked.gateway,
                serving_and_revoked.old_service,
            )
            .await
            .expect("accepted invocation count"),
        1
    );
    assert_eq!(
        store
            .count_accepted_service_invocations(
                serving_and_revoked.gateway,
                serving_and_revoked.candidate_service,
            )
            .await
            .expect("candidate invocation count"),
        0
    );
    assert_eq!(
        store
            .count_accepted_service_invocations(serving_and_revoked.gateway, mixed.old_service,)
            .await
            .expect("project/revision mismatch count"),
        0
    );
    let old_key = GatewayServiceInstanceKey {
        identity: GatewayServiceIdentity {
            instance_id: serving_and_revoked.old_instance,
            gateway_id: serving_and_revoked.gateway,
            revision_id: serving_and_revoked.old_service,
        },
        fencing_token: 1,
    };
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(old_key)
            .await
            .expect("exact accepted invocation count"),
        1
    );
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(GatewayServiceInstanceKey {
                fencing_token: 2,
                ..old_key
            })
            .await
            .expect("stale fence accepted invocation count"),
        0
    );
    let mixed_key = GatewayServiceInstanceKey {
        identity: GatewayServiceIdentity {
            instance_id: mixed.old_instance,
            gateway_id: mixed.gateway,
            revision_id: mixed.old_service,
        },
        fencing_token: 1,
    };
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(mixed_key)
            .await
            .expect("different instance accepted invocation count"),
        1
    );
    assert_eq!(
        store
            .count_accepted_service_invocations_for_instance(old_key)
            .await
            .expect("original instance count remains isolated"),
        1
    );
    assert!(
        store
            .count_accepted_service_invocations_for_instance(GatewayServiceInstanceKey {
                identity: GatewayServiceIdentity {
                    instance_id: Uuid::nil(),
                    ..old_key.identity
                },
                fencing_token: 1,
            })
            .await
            .is_err()
    );
}
