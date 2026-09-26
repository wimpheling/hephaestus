//! Recovery-owner and browser-session fixture helpers.

use super::super::super::rpc::update_admission::{
    RecoveryAction, UpdateAdmissionInstance, UpdateAdmissionResult, UpdateRpcClient, recover_update,
};
use identity_domain::{
    AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};

use super::lifecycle::wait_for_new_hook_run;

/// Authorizes a new owner and verifies explicit recovery gets a fresh hook ID.
pub async fn recover_update_with_new_owner(
    pool: &sqlx::PgPool,
    client: &UpdateRpcClient,
    instance: &UpdateAdmissionInstance,
    update_id: uuid::Uuid,
    hook_run_id: uuid::Uuid,
    retry_hook_run_id: uuid::Uuid,
) -> UpdateAdmissionResult {
    // A different currently authorized owner can explicitly recover the
    // fenced update. The recovery actor is persisted with the update so
    // subsequent durable admission reauthorizes that owner rather than the
    // revoked creator.
    let recovery_actor = uuid::Uuid::new_v4();
    let recovery_sid = BrowserSessionSid::new();
    let project_id: uuid::Uuid =
        sqlx::query_scalar("SELECT project_id FROM agent_instances WHERE id = $1")
            .bind(instance.instance_id)
            .fetch_one(pool)
            .await
            .expect("load update project for recovery owner");
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Recovery Owner')")
        .bind(recovery_actor)
        .execute(pool)
        .await
        .expect("seed recovery owner");
    seed_update_browser_session(pool, recovery_actor, recovery_sid).await;
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project_id)
        .bind(recovery_actor)
        .execute(pool)
        .await
        .expect("authorize recovery owner");
    recover_update(
        client,
        recovery_actor,
        recovery_sid,
        update_id,
        RecoveryAction::Retry,
        "app-update-admission-owner-retry",
    )
    .await;
    let owner_retry_hook_run_id =
        wait_for_new_hook_run(pool, update_id, Some(retry_hook_run_id)).await;
    assert_ne!(owner_retry_hook_run_id, hook_run_id);
    assert_ne!(owner_retry_hook_run_id, retry_hook_run_id);
    let persisted_actor: uuid::Uuid =
        sqlx::query_scalar("SELECT actor_id FROM agent_updates WHERE id = $1")
            .bind(update_id)
            .fetch_one(pool)
            .await
            .expect("inspect persisted recovery owner");
    assert_eq!(persisted_actor, recovery_actor);
    UpdateAdmissionResult {
        update_id,
        initial_hook_run_id: hook_run_id,
        retried_hook_run_id: retry_hook_run_id,
        owner_recovery_hook_run_id: owner_retry_hook_run_id,
    }
}

pub async fn seed_update_browser_session(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
    sid: BrowserSessionSid,
) {
    let verified = AuthenticatedIdentity::new(
        UserId::from_uuid(user_id),
        "https://issuer.golden.invalid",
        format!("update-{user_id}"),
        serde_json::Value::Null,
        RequestId::new(),
    );
    sqlx::query(
        "INSERT INTO human_browser_sessions
            (id, sid_digest, creation_idempotency_id, creation_request_id,
             identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(browser_session_sid_digest(sid).as_bytes().to_vec())
    .bind(uuid::Uuid::new_v4())
    .bind(uuid::Uuid::new_v4())
    .bind(
        browser_session_identity_binding_digest(&verified)
            .as_bytes()
            .to_vec(),
    )
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed update recovery browser session");
}
