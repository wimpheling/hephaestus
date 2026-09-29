use super::*;

#[tokio::test]
#[serial]
// This deliberately crosses every durable boundary in one scenario.
#[allow(clippy::too_many_lines)]
async fn pushed_config_publishes_command_and_starts_vm() {
    let (Ok(nats_url), Some((pool, service, repository, temporary))) =
        (std::env::var("HEPHAESTUS_NATS_TEST_URL"), fixture().await)
    else {
        return;
    };
    // The publisher scans the shared forge outbox, so pre-existing fixtures
    // must not supply an older StartRun delivery to this scenario's consumer.
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'forge' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate forge start-run fixture");
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (_, update) = commit_and_update(&temporary, &repository, &config).await;
    let receive = service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("accepted receive");
    let request = receive.run_requests[0].clone();

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let consumer = ensure_jetstream_topology(&context)
        .await
        .expect("run topology");
    ensure_forge_jetstream_topology(&context)
        .await
        .expect("forge topology");

    let run_repository = Arc::new(PgRunRepository::new(pool.clone()));
    let volume_root = temporary.path().join("volumes");
    let volumes = Arc::new(
        LocalVolumeStore::new(
            Arc::new(PostgresVolumeMetadataRepository::new(pool.clone())),
            LocalVolumeConfig {
                volume_root,
                transient_runtime_roots: Vec::new(),
                host_id: String::from("phase2-integration"),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: std::path::PathBuf::from("/usr/bin/mkfs.ext4"),
            },
        )
        .expect("volume configuration"),
    );
    volumes.initialize().await.expect("volume store");
    let guest_root = temporary.path().join("guest-root");
    tokio::fs::create_dir(&guest_root)
        .await
        .expect("guest root");
    let orchestrator = Arc::new(RunOrchestrator::new(
        run_repository.clone(),
        volumes,
        Arc::new(FakeProvider::new()),
        Arc::new(TestSpecFactory { root: guest_root }),
        16 * 1024 * 1024,
    ));
    let handler = NatsCommandHandler::new(Arc::clone(&orchestrator));
    let publisher = ForgeNatsOutboxPublisher::new(context.clone());
    assert!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("publish receive commands")
            > 0
    );

    let mut messages = consumer.messages().await.expect("command messages");
    let message = messages
        .next()
        .await
        .expect("start delivery")
        .expect("valid start delivery");
    let handler_task = tokio::spawn(async move { handler.handle(&message).await });
    wait_for_run_state(&pool, request.command.run_id, "running").await;
    orchestrator
        .cancel_run(&CancelRun {
            command_id: CommandId::new(),
            run_id: request.command.run_id,
            reason: String::from("complete integration test"),
        })
        .await
        .expect("stop started VM");
    handler_task
        .await
        .expect("join command handler")
        .expect("handle start command");
    assert_eq!(
        run_repository
            .get(request.command.run_id)
            .await
            .expect("completed run")
            .state,
        RunState::CleanedUp
    );
    let reached_running: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM run_events
           WHERE run_id = $1 AND event_type = 'run.running'
         )",
    )
    .bind(request.command.run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("running transition");
    assert!(reached_running, "published command did not start the VM");

    cleanup_run(&pool, &request.command).await;
    cleanup(&pool, repository).await;
    context
        .delete_stream("HEPH_RUN_COMMANDS")
        .await
        .expect("delete command stream");
    context
        .delete_stream("HEPHAESTUS_GIT_EVENTS")
        .await
        .expect("delete Git event stream");
}
