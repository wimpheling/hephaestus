//! uuid pages scenario.

use super::support::{seed_gateway, set_pointers, test_pool, worker_pool};
use gateway_domain::{GatewayServiceTargetPage, GatewayServiceTargetStore};
use gateway_postgres::PostgresGatewayServiceTargets;
use serial_test::serial;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn service_target_uuid_pages_remain_stable_across_concurrent_writes() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let worker = worker_pool().await;
    let store = PostgresGatewayServiceTargets::new(worker);
    let first_fixture = seed_gateway(
        &pool,
        &format!("pagination-scalar-a-{}", Uuid::new_v4()),
        "enabled",
        "published",
    )
    .await;
    set_pointers(
        &pool,
        first_fixture.gateway,
        first_fixture.old_service,
        Some(first_fixture.candidate_service),
    )
    .await;
    let second_fixture = seed_gateway(
        &pool,
        &format!("pagination-scalar-b-{}", Uuid::new_v4()),
        "enabled",
        "published",
    )
    .await;
    set_pointers(
        &pool,
        second_fixture.gateway,
        second_fixture.old_service,
        Some(second_fixture.candidate_service),
    )
    .await;
    let third_fixture = seed_gateway(
        &pool,
        &format!("pagination-scalar-c-{}", Uuid::new_v4()),
        "enabled",
        "published",
    )
    .await;
    set_pointers(
        &pool,
        third_fixture.gateway,
        third_fixture.old_service,
        Some(third_fixture.candidate_service),
    )
    .await;
    let expected = [
        first_fixture.gateway,
        second_fixture.gateway,
        third_fixture.gateway,
    ];

    let first = store
        .list_service_targets(GatewayServiceTargetPage::new(None, 2).expect("first target page"))
        .await
        .expect("first service target page");
    let first_cursor = first.next_after.expect("more targets after first page");

    let (release_writer, writer_released) = tokio::sync::oneshot::channel();
    let writer_pool = pool.clone();
    let writer = tokio::spawn(async move {
        writer_released
            .await
            .expect("release concurrent target writer");
        let fixture = seed_gateway(
            &writer_pool,
            &format!("ps-inserted-{}", Uuid::new_v4()),
            "enabled",
            "published",
        )
        .await;
        set_pointers(
            &writer_pool,
            fixture.gateway,
            fixture.old_service,
            Some(fixture.candidate_service),
        )
        .await;
        fixture.gateway
    });
    release_writer
        .send(())
        .expect("release concurrent target writer");
    let inserted = writer.await.expect("concurrent target writer task");

    let mut observed = first.targets;
    let mut cursor = Some(first_cursor);
    while let Some(after) = cursor {
        let page = store
            .list_service_targets(
                GatewayServiceTargetPage::new(Some(after), 2).expect("following target page"),
            )
            .await
            .expect("following service target page");
        assert!(page.targets.iter().all(|target| target.gateway_id > after));
        observed.extend(page.targets);
        cursor = page.next_after;
    }

    assert!(
        observed
            .windows(2)
            .all(|pair| pair[0].gateway_id < pair[1].gateway_id)
    );
    for gateway in expected {
        assert_eq!(
            observed
                .iter()
                .filter(|target| target.gateway_id == gateway)
                .count(),
            1,
            "preexisting gateway must appear exactly once across pages"
        );
    }
    let inserted_count = observed
        .iter()
        .filter(|target| target.gateway_id == inserted)
        .count();
    assert_eq!(
        inserted_count,
        usize::from(inserted > first_cursor),
        "inter-page insert follows the exclusive UUID cursor contract"
    );
}
