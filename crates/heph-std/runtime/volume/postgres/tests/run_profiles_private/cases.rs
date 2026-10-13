use sqlx::PgPool;
use uuid::Uuid;

use super::{
    fixture::{self, Execution, Mutation},
    producers,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn upgrade_preserves_existing_rows_and_checksums_without_owner_adoption(pool: PgPool) {
    let f = fixture::baseline(&pool).await;
    let mut history = vec![(f.run, fixture::snapshot(&pool, f.run).await)];
    for state in ["running", "cleaned_up"] {
        let run = Uuid::new_v4();
        sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,state,requires_state,created_at,updated_at) SELECT $1,instance_id,instance_revision_id,release_id,release_agent_id,$2,'normal',gen_random_uuid(),$3,requires_state,created_at,updated_at FROM runs WHERE id=$4")
            .bind(run).bind(f.attachment).bind(state).bind(f.run).execute(&pool).await.unwrap();
        history.push((run, fixture::snapshot(&pool, run).await));
    }
    let before = fixture::snapshot(&pool, f.run).await;
    let checksums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    fixture::checkpoint(108).run(&pool).await.unwrap();
    assert_eq!(fixture::snapshot(&pool, f.run).await, before);
    for (run, row) in history {
        assert_eq!(fixture::snapshot(&pool, run).await, row);
        fixture::denied(&pool, run, Mutation::Promote).await;
    }
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(checksums, after);
    let profile: String = sqlx::query_scalar("SELECT execution_profile FROM runs WHERE id=$1")
        .bind(f.run)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(profile, "historical_unproven");
    for mutation in [
        Mutation::Progress,
        Mutation::Bind,
        Mutation::Scope,
        Mutation::Promote,
        Mutation::Clean,
    ] {
        fixture::denied(&pool, f.run, mutation).await;
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn unspecified_fresh_run_is_closed_history_not_execution_permission(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("INSERT INTO runs(id,command_id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,requires_state,state,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,'normal',true,'queued',now(),now())")
        .bind(e.run).bind(e.command).bind(e.instance).bind(e.revision).bind(e.release).bind(e.agent).bind(e.attachment).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    fixture::denied(&pool, e.run, Mutation::Progress).await;
    fixture::denied(&pool, e.run, Mutation::Scope).await;
    assert!(fixture::lease(&pool, &e, &f).await.is_err());
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn worker_legacy_git_commits_and_binds_only_in_provisioning(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    producers::normal(&pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::denied(&pool, e.run, Mutation::Bind).await;
    fixture::advance(&pool, e.run, "leasing_volume").await;
    let mut actor = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *actor)
        .await
        .unwrap();
    let actor_result = sqlx::query("UPDATE runs SET vm_id=id::text WHERE id=$1")
        .bind(e.run)
        .execute(&mut *actor)
        .await;
    match actor_result {
        Ok(result) => assert_eq!(result.rows_affected(), 0, "actor must not bind any Run"),
        Err(error) => assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("42501")
        ),
    }
    actor.rollback().await.unwrap();
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("UPDATE runs SET vm_id=id::text WHERE id=$1")
        .bind(e.run)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    fixture::denied(&pool, e.run, Mutation::Scope).await;
    let later = Execution::stateless(&pool, &f).await;
    producers::normal(&pool, &later, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::advance(&pool, later.run, "running").await;
    fixture::denied(&pool, later.run, Mutation::Bind).await;
    fixture::advance(&pool, later.run, "cleaning_up").await;
    fixture::denied(&pool, later.run, Mutation::Bind).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn worker_legacy_accepted_mailbox_commits_without_git_request(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    let mut tx = fixture::worker(&pool).await;
    fixture::insert(&mut tx, &e, "legacy_scalar", false, false)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE NONE")
        .execute(&mut *tx)
        .await
        .unwrap();
    producers::mailbox(&mut tx, &e, &f).await;
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE run_id=$1")
        .bind(e.run)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    // Trusted fixture snapshot metadata tests an allowed status update. No
    // bearer session, binding grant, VM or runtime authorization is created.
    let snapshot = Uuid::new_v4();
    sqlx::query("INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash) VALUES($1,$2,$3,$4,'fixture/v1',$5)")
        .bind(snapshot).bind(e.run).bind(e.instance).bind(e.revision).bind([7_u8;32].as_slice()).execute(&pool).await.unwrap();
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("UPDATE mailbox_delivery_attempts SET state='running',authorization_snapshot_id=$2 WHERE run_id=$1")
        .bind(e.run).bind(snapshot)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn worker_legacy_update_commits_exact_hook_without_owned_admission(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let (e, result) = producers::update(&pool, &f, false).await;
    result.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_vm_plan_admissions WHERE run_id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    fixture::advance(&pool, e.run, "provisioning").await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn actor_wrong_requirement_and_missing_producer_cannot_profile_runs(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        fixture::insert(&mut tx, &e, "legacy_scalar", false, false)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let mut wrong = e.clone();
    wrong.requires_state = false;
    assert!(
        producers::normal(&pool, &wrong, "legacy_scalar", false)
            .await
            .is_err()
    );
    let mut tx = fixture::worker(&pool).await;
    fixture::insert(&mut tx, &e, "legacy_scalar", false, false)
        .await
        .unwrap();
    assert!(tx.commit().await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE id=$1")
        .bind(e.run)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn profile_and_positive_producer_pins_are_immutable(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    producers::normal(&pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    for mutation in [Mutation::Promote, Mutation::Pin, Mutation::Scope] {
        fixture::denied(&pool, e.run, mutation).await;
    }
    let mut tx = pool.begin().await.unwrap();
    assert!(
        sqlx::query("DELETE FROM command_inbox WHERE command_id=$1")
            .bind(e.command)
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(
        sqlx::query("UPDATE run_requests SET commit_sha=$2 WHERE run_id=$1")
            .bind(e.run)
            .bind("b".repeat(40))
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn owned_zero_requires_exact_run_scope_even_with_real_git_evidence(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::stateless(&pool, &f).await;
    assert!(
        producers::normal(&pool, &e, "owned_canonical", false)
            .await
            .is_err()
    );
    producers::normal(&pool, &e, "owned_canonical", true)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "cleaning_up").await;
    fixture::denied(&pool, e.run, Mutation::Clean).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn owned_mailbox_cannot_borrow_matching_git_admission(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    let mut tx = pool.begin().await.unwrap();
    producers::git(&mut tx, &e).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    fixture::insert(&mut tx, &e, "owned_canonical", true, false)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE NONE")
        .execute(&mut *tx)
        .await
        .unwrap();
    producers::mailbox(&mut tx, &e, &f).await;
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(tx.commit().await.is_err());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE run_id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn named_unsupported_gate_cannot_use_legacy_profile(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let mut e = Execution::normal(&pool, &f).await;
    e.instance = Uuid::new_v4();
    e.revision = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state,volume_mode,run_gate_open) SELECT $1,project_id,family_id,$2,'active','named',false FROM agent_instances WHERE id=$3").bind(e.instance).bind(e.instance.to_string()).bind(f.instance).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,$2,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=(SELECT active_revision_id FROM agent_instances WHERE id=$3)")
        .bind(e.revision).bind(e.instance).bind(f.instance).execute(&pool).await.unwrap();
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(e.instance)
        .bind(e.revision)
        .execute(&pool)
        .await
        .unwrap();
    let attachment = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_attachments(id,instance_id,project_id,repository_id,ref_selector,trigger_policy) VALUES($1,$2,$3,$4,'refs/heads/main','manual')")
        .bind(attachment).bind(e.instance).bind(f.consuming_project).bind(f.consuming_repository).execute(&pool).await.unwrap();
    e.attachment = Some(attachment);
    assert!(
        producers::normal(&pool, &e, "legacy_scalar", false)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn historical_lease_fences_cannot_renew_attach_or_release(pool: PgPool) {
    let f = fixture::baseline(&pool).await;
    let unknown = super::history::update_only(&pool, &f).await;
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES(gen_random_uuid(),$1,$2,$3,'host',7,'active',now()+interval '1 hour')")
        .bind(f.volume).bind(f.instance).bind(f.run).execute(&pool).await.unwrap();
    let before: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(l) FROM agent_instance_volume_leases l WHERE run_id=$1",
    )
    .bind(f.run)
    .fetch_one(&pool)
    .await
    .unwrap();
    fixture::checkpoint(108).run(&pool).await.unwrap();
    super::history::assert_held(&pool, unknown).await;
    for mutation in [Mutation::Attached, Mutation::Heartbeat, Mutation::Release] {
        fixture::denied(&pool, f.run, mutation).await;
    }
    let after:serde_json::Value=sqlx::query_scalar("SELECT to_jsonb(l)-'selection_origin'-'selected_revision_id'-'selected_release_id'-'selected_release_agent_id'-'selected_project_id'-'slot_key'-'access_mode'-'guest_path'-'slot_required'-'minimum_capacity_bytes'-'release_contract_hash'-'named_binding_revision_id' FROM agent_instance_volume_leases l WHERE run_id=$1").bind(f.run).fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
}

#[path = "cleanup_cases.rs"]
mod cleanup_cases;
