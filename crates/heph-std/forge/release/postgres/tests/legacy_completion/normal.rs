use super::fixture;
use run_domain::RunKind;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn normal_and_mailbox_terminal_callbacks_use_exact_scoped_producers(pool: PgPool) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    for mailbox in [false, true] {
        let run = fixture::fresh(&pool, &worker, &scope, false, mailbox).await;
        fixture::running(&worker, &run).await;
        assert!(
            fixture::service(&worker, &scope)
                .legacy_run_cleanup_completed(run.run_id)
                .await
                .is_err()
        );
        fixture::finish(&worker, &scope, &run).await;
        let before = fixture::snapshot(&pool, &run).await;
        assert_eq!(
            fixture::service(&worker, &scope)
                .legacy_run_cleanup_completed(run.run_id)
                .await
                .unwrap(),
            RunKind::Normal
        );
        assert_eq!(fixture::snapshot(&pool, &run).await, before);
    }
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn foreign_scope_actor_role_and_unconfigured_history_cannot_claim_completion(pool: PgPool) {
    // Genuine public107 history is grandfathered without fabricating a new
    // unspecified terminal Run after the private positive-birth guards exist.
    fixture::public_schema(&pool).await;
    let historical = fixture::command();
    super::support::seed_instance(&pool, &historical).await;
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,command_id,run_kind,state,requires_state,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,'normal','cleaned_up',false,now(),now())")
        .bind(historical.run_id.as_uuid()).bind(historical.instance_id.as_uuid())
        .bind(historical.instance_revision_id.as_uuid()).bind(historical.release_id.as_uuid())
        .bind(historical.release_agent_id.as_uuid()).bind(historical.attachment_id.unwrap().as_uuid())
        .bind(historical.command_id.as_uuid()).execute(&pool).await.unwrap();
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let run = fixture::fresh(&pool, &worker, &scope, false, false).await;
    fixture::running(&worker, &run).await;
    fixture::finish(&worker, &scope, &run).await;
    let before = fixture::snapshot(&pool, &run).await;
    assert!(
        fixture::service(&worker, &fixture::owner())
            .legacy_run_cleanup_completed(run.run_id)
            .await
            .is_err()
    );
    let actor = fixture::role(&pool, false).await;
    assert!(
        fixture::service(&actor, &scope)
            .legacy_run_cleanup_completed(run.run_id)
            .await
            .is_err()
    );
    let default = release_postgres::ReleaseService::new(
        worker.clone(),
        std::sync::Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    );
    assert!(
        default
            .legacy_run_cleanup_completed(run.run_id)
            .await
            .is_err()
    );
    drop(default);
    assert_eq!(fixture::snapshot(&pool, &run).await, before);
    let old = fixture::snapshot(&pool, &historical).await;
    assert!(
        fixture::service(&worker, &scope)
            .legacy_run_cleanup_completed(historical.run_id)
            .await
            .is_err()
    );
    assert_eq!(fixture::snapshot(&pool, &historical).await, old);
    actor.close().await;
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn persistent_destroy_failure_boundary_holds_original_fences_and_rejects_callback(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let run = fixture::fresh(&pool, &worker, &scope, true, false).await;
    let lease = fixture::acquire(&pool, &worker, &scope, &run, None).await;
    fixture::running(&worker, &run).await;
    fixture::close(&worker, &scope, &run).await;
    fixture::outcome(&worker, &run, false).await;
    // The production Core persistent-destroy case is separately validated;
    // here its closed CleaningUp/held-fence boundary must not admit a callback.
    let before = fixture::snapshot(&pool, &run).await;
    assert!(
        fixture::service(&worker, &scope)
            .legacy_run_cleanup_completed(run.run_id)
            .await
            .is_err()
    );
    assert_eq!(before["run"]["state"], "cleaning_up");
    assert_eq!(before["leases"][0]["id"], lease.id.to_string());
    assert_eq!(before["leases"][0]["released_at"], serde_json::Value::Null);
    assert_eq!(fixture::snapshot(&pool, &run).await, before);
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn released_callback_replay_never_resets_newer_consumer_generation(pool: PgPool) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let first = fixture::fresh(&pool, &worker, &scope, true, false).await;
    let old = fixture::acquire(&pool, &worker, &scope, &first, None).await;
    fixture::running(&worker, &first).await;
    fixture::close(&worker, &scope, &first).await;
    fixture::outcome(&worker, &first, true).await;
    fixture::detach(&worker, &scope, &first, &old).await;
    fixture::transition(&worker, &first, "cleaned_up").await;
    let next = super::support::retry_command(&first);
    fixture::birth(&pool, &worker, &scope, &next, false).await;
    let current = fixture::acquire(&pool, &worker, &scope, &next, Some(old.volume_id)).await;
    assert!(current.fencing_token > old.fencing_token);
    let before = fixture::snapshot(&pool, &first).await;
    assert_eq!(
        fixture::service(&worker, &scope)
            .legacy_run_cleanup_completed(first.run_id)
            .await
            .unwrap(),
        RunKind::Normal
    );
    assert_eq!(fixture::snapshot(&pool, &first).await, before);
    worker.close().await;
}
