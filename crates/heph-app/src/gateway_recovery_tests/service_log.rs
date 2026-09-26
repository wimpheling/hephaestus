use super::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn production_service_log_writer_persists_vm_events_and_respects_disabled_mode() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let worker = database.worker.clone();
    let application = seed_application_log_fixture(&pool, "http.service.v1").await;
    let store: Arc<dyn GatewayServiceLogStore> =
        Arc::new(PostgresGatewayServiceLogStore::new(worker.clone()));
    let writer = GatewayServiceLogWriterConfig::new(store, ServiceLogWriterPolicy::default());
    let (
        application_task,
        application_cancellation,
        application_caddy_started,
        application_caddy_release,
        _application_destroyed,
        _application_provisioned,
        application_events,
    ) = spawn_automatic_start_with_worker_config(
        &pool,
        &worker,
        application,
        false,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(Arc::new(ApplicationLogLaunchResolver)),
        None,
        Some(writer.clone()),
    )
    .await;
    tokio::time::timeout(
        StdDuration::from_secs(10),
        application_caddy_started.notified(),
    )
    .await
    .expect("application reconciliation starts");
    application_caddy_release.notify_one();
    assert!(wait_for_ready(&pool, application).await);
    let application_instance: (Uuid, i64) = sqlx::query_as(
        "SELECT id, fencing_token
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(application.gateway)
    .bind(application.revision)
    .fetch_one(&pool)
    .await
    .expect("read application instance lease");
    let application_sender = tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let sender = application_events
                .lock()
                .expect("application event sender")
                .clone();
            if let Some(sender) = sender {
                break sender;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("application VM event subscriber");
    application_sender
        .send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"application-ready-event".to_vec(),
        })
        .expect("application event subscriber remains active");
    tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*)
                   FROM gateway_service_log_chunks
                  WHERE instance_id = $1 AND fencing_token = $2",
            )
            .bind(application_instance.0)
            .bind(application_instance.1)
            .fetch_one(&pool)
            .await
            .expect("read application log chunks");
            if count > 0 {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(50)).await;
        }
    })
    .await
    .expect("application event persisted before shutdown");
    application_cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), application_task)
        .await
        .expect("application reconciliation shutdown")
        .expect("application reconciliation task");
    let (application_epoch_count, application_cleaned): (i64, String) = sqlx::query_as(
        "SELECT
                (SELECT count(*) FROM gateway_service_log_epochs
                  WHERE instance_id = $1 AND fencing_token = $2),
                (SELECT state FROM gateway_service_instances WHERE id = $1)",
    )
    .bind(application_instance.0)
    .bind(application_instance.1)
    .fetch_one(&pool)
    .await
    .expect("read final application log state");
    assert_eq!(application_epoch_count, 1);
    let application_rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT sequence, stream, bytes
           FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND fencing_token = $2
          ORDER BY sequence",
    )
    .bind(application_instance.0)
    .bind(application_instance.1)
    .fetch_all(&pool)
    .await
    .expect("read ordered application log chunks");
    assert_eq!(application_rows.len(), 2);
    assert!(application_rows[0].0 < application_rows[1].0);
    assert_eq!(application_rows[0].1, "stdout");
    assert_eq!(application_rows[0].2, b"application-ready-event");
    assert_eq!(application_rows[1].1, "stderr");
    assert_eq!(application_rows[1].2, b"application-final-event");
    assert_eq!(application_cleaned, "cleaned");
    application_caddy_release.notify_one();
    clear_service_log_fixture(&pool, application).await;
    cleanup_startup_fixture(&pool, application).await;
    drop_isolated_startup_database(database).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn production_service_log_writer_skips_disabled_vm_events() {
    let Some(database) = isolated_startup_database().await else {
        return;
    };
    let pool = database.control.clone();
    let worker = database.worker.clone();
    let disabled = seed_fixture(&pool, "http.service.v1").await;
    let disabled_store: Arc<dyn GatewayServiceLogStore> =
        Arc::new(PostgresGatewayServiceLogStore::new(worker.clone()));
    let disabled_writer =
        GatewayServiceLogWriterConfig::new(disabled_store, ServiceLogWriterPolicy::default());
    let (
        disabled_task,
        disabled_cancellation,
        disabled_caddy_started,
        disabled_caddy_release,
        _disabled_destroyed,
        _disabled_provisioned,
        disabled_events,
    ) = spawn_automatic_start_with_worker_config(
        &pool,
        &worker,
        disabled,
        false,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(disabled_writer),
    )
    .await;
    tokio::time::timeout(
        StdDuration::from_secs(10),
        disabled_caddy_started.notified(),
    )
    .await
    .expect("disabled reconciliation starts");
    disabled_caddy_release.notify_one();
    assert!(wait_for_ready(&pool, disabled).await);
    let disabled_instance: Uuid = sqlx::query_scalar(
        "SELECT id
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2 AND state = 'ready'
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(disabled.gateway)
    .bind(disabled.revision)
    .fetch_one(&pool)
    .await
    .expect("read disabled instance");
    let disabled_sender = tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let sender = disabled_events
                .lock()
                .expect("disabled event sender")
                .clone();
            if let Some(sender) = sender {
                break sender;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("disabled VM event subscriber");
    disabled_sender
        .send(VmEvent::Log {
            stream: vm_trait::LogStream::Stdout,
            bytes: b"disabled-event".to_vec(),
        })
        .expect("disabled event subscriber remains active");
    disabled_cancellation.cancel();
    tokio::time::timeout(StdDuration::from_secs(10), disabled_task)
        .await
        .expect("disabled reconciliation shutdown")
        .expect("disabled reconciliation task");
    let disabled_epochs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_service_log_epochs WHERE instance_id = $1",
    )
    .bind(disabled_instance)
    .fetch_one(&pool)
    .await
    .expect("read disabled log epochs");
    assert_eq!(disabled_epochs, 0);
    disabled_caddy_release.notify_one();
    clear_service_log_fixture(&pool, disabled).await;
    cleanup_startup_fixture(&pool, disabled).await;
    drop_isolated_startup_database(database).await;
}
