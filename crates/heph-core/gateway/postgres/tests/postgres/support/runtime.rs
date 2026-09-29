//! Runtime service instance fixture helpers.

use super::fixture::Fixture;
use uuid::Uuid;

pub async fn seed_dispatch_target(pool: &sqlx::PgPool, fixture: &Fixture) {
    let repository: Uuid = sqlx::query_scalar("SELECT repository_id FROM gateways WHERE id = $1")
        .bind(fixture.gateway)
        .fetch_one(pool)
        .await
        .expect("fixture repository");
    let receive = Uuid::new_v4();
    sqlx::query("INSERT INTO git_receives (id, repository_id, principal, status, accepted_at) VALUES ($1, $2, 'gateway-test', 'accepted', now())")
        .bind(receive).bind(repository).execute(pool).await.expect("receive");
    sqlx::query("INSERT INTO git_refs (repository_id, git_ref, commit_sha, updated_by_receive_id) VALUES ($1, 'refs/heads/main', $2, $3)")
        .bind(repository).bind("a".repeat(40)).bind(receive).execute(pool).await.expect("target ref");
    sqlx::query("INSERT INTO agent_attachments (id, instance_id, project_id, repository_id, ref_selector, trigger_policy) SELECT gen_random_uuid(), instance_id, project_id, $2, 'refs/heads/main', 'manual' FROM mailboxes WHERE id = $1")
        .bind(fixture.mailbox).bind(repository).execute(pool).await.expect("runnable attachment");
}

pub async fn set_instance_state(pool: &sqlx::PgPool, mailbox: Uuid, state: &str) {
    sqlx::query("UPDATE agent_instances SET state = $2, run_gate_open = false, removed_at = CASE WHEN $2 = 'removed' THEN now() ELSE NULL END WHERE id = (SELECT instance_id FROM mailboxes WHERE id = $1)")
        .bind(mailbox).bind(state).execute(pool).await.expect("set fixture lifecycle");
}
