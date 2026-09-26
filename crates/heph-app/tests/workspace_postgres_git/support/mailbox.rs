use super::*;

// This regression traverses both production adapters over one actual mailbox
// attempt; keeping its complete FK graph together makes the snapshot auditable.
#[allow(clippy::too_many_lines)]
pub async fn assert_mailbox_target_survives_ref_movement(
    pool: &PgPool,
    original: &StartRun,
    accepted: &CommitSha,
    newer: &CommitSha,
    project: Uuid,
    repository: Uuid,
) -> run_domain::Run {
    let command = StartRun {
        command_id: runtime_types::CommandId::new(),
        run_id: runtime_types::RunId::new(),
        ..original.clone()
    };
    let runs = PgRunRepository::new(pool.clone());
    let run = runs
        .create_run(&command)
        .await
        .expect("mailbox run without Git receive")
        .run;
    let mailbox = Uuid::new_v4();
    let event = Uuid::new_v4();
    let body = Uuid::new_v4();
    sqlx::query("INSERT INTO mailboxes(id,project_id,instance_id,state) VALUES($1,$2,$3,'active')")
        .bind(mailbox)
        .bind(project)
        .bind(command.instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("mailbox");
    sqlx::query("INSERT INTO mailbox_payloads(id,mailbox_id,project_id,encoded_body,encoded_length,decoded_length,integrity_hash) VALUES($1,$2,$3,'x'::bytea,1,1,decode('2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881','hex'))")
        .bind(body).bind(mailbox).bind(project).execute(pool).await.expect("mailbox body");
    sqlx::query("INSERT INTO mailbox_events(id,mailbox_id,project_id,instance_id,body_id,producer_kind,producer_id,deduplication_scope,deduplication_key,method,route,received_at) VALUES($1,$2,$3,$4,$5,'system','runtime-test','test','snapshot','POST','/test',now())")
        .bind(event).bind(mailbox).bind(project).bind(command.instance_id.as_uuid()).bind(body)
        .execute(pool).await.expect("mailbox event");
    sqlx::query("INSERT INTO mailbox_delivery_attempts(id,event_id,mailbox_id,attempt_number,state,command_id,instance_id,instance_revision_id,run_id,target_ref,target_commit,state_access_outcome) VALUES($1,$2,$3,1,'leased',$4,$5,$6,$7,'refs/heads/main',$8,'uncertain_access')")
        .bind(Uuid::new_v4()).bind(event).bind(mailbox).bind(command.command_id.as_uuid())
        .bind(command.instance_id.as_uuid()).bind(command.instance_revision_id.as_uuid())
        .bind(command.run_id.as_uuid()).bind(accepted.as_str())
        .execute(pool).await.expect("claimed immutable mailbox target");
    let requests: i64 = sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE run_id=$1")
        .bind(run.id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("no Git request");
    assert_eq!(requests, 0);
    sqlx::query(
        "UPDATE git_refs SET commit_sha=$2 WHERE repository_id=$1 AND git_ref='refs/heads/main'",
    )
    .bind(repository)
    .bind(newer.as_str())
    .execute(pool)
    .await
    .expect("move live source ref");
    let runtime = runs
        .load_runtime(&run)
        .await
        .expect("mailbox exact runtime target");
    assert_eq!(runtime.repository_id, Some(repository));
    assert_eq!(runtime.git_ref.as_deref(), Some("refs/heads/main"));
    assert_eq!(runtime.commit_sha.as_deref(), Some(accepted.as_str()));
    assert_eq!(runtime.mailbox_event.expect("mailbox payload").body, b"x");
    let workspaces = PgWorkspaceMetadataRepository::new(pool.clone());
    let workspace = workspaces
        .request(command.command_id.as_uuid())
        .await
        .expect("mailbox workspace lookup")
        .expect("mailbox workspace target");
    assert_eq!(workspace.repository_id, repository);
    assert_eq!(workspace.commit_sha, accepted.as_str());
    assert_ne!(workspace.commit_sha, newer.as_str());
    run
}

pub async fn assert_mailbox_result_proposal(
    pool: &PgPool,
    manager: &LocalWorkspaceManager,
    run: &run_domain::Run,
    accepted: &CommitSha,
) {
    let prepared = manager.prepare(run).await.expect("mailbox exact workspace");
    let writable = prepared
        .mounts
        .iter()
        .find(|mount| mount.tag == "repository-work")
        .expect("mailbox writable proposal tree");
    assert_eq!(
        tokio::fs::read_to_string(writable.host_path.join("input.txt"))
            .await
            .expect("captured source"),
        "accepted\n"
    );
    tokio::fs::write(writable.host_path.join("input.txt"), "mailbox proposal\n")
        .await
        .expect("mailbox edit");
    let result = manager
        .finalize(run, "mailbox result")
        .await
        .expect("controlled mailbox result")
        .expect("mailbox result exists");
    let proposal: (String, String, String) = sqlx::query_as(
        "SELECT target_ref,input_commit,result_commit FROM review_proposals WHERE run_id=$1",
    )
    .bind(run.id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("mailbox review proposal exists");
    assert_eq!(proposal.0, "refs/heads/main");
    assert_eq!(proposal.1, accepted.as_str());
    assert_eq!(proposal.2, result.result_commit);
    manager
        .finalize(run, "retry")
        .await
        .expect("idempotent mailbox result");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM review_proposals WHERE run_id=$1")
        .bind(run.id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("proposal count");
    assert_eq!(count, 1);
}
