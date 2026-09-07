//! Bounded cooking update and recovery probes.
//!
//! The helper deliberately takes IDs returned by the real build/release path.
//! It does not insert a release, update, revision, or successful run row.  A
//! golden caller supplies ordinary published candidates (migration, explicit
//! rejection, and an abnormal hook) and may then send ingress while the gate
//! is closed before awaiting the assertions below.

use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceServiceClient,
    messages::hephaestus::{
        common::v1::{NetworkPolicy, OpaqueId, RequestContext, RuntimePolicy},
        instance::v1::{
            BrokeredRuleCopy, CreateUpdateRequest, RecoverUpdateRequest, RecoveryAction,
        },
    },
};
use secret_application::RotateSecret;
use secret_domain::{SecretCommandKey, SecretId, SecretValue, SecretVersionId};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use sqlx::PgPool;
use std::{path::Path, time::Duration};
use tokio::time::{Instant, sleep, timeout};
use uuid::Uuid;

/// Authentication and durable resources used by the update RPC helpers.
pub struct CookingUpdateContext<'a> {
    /// Application database used for lifecycle and deferred-work assertions.
    pub pool: &'a PgPool,
    /// Running daemon exposing the instance Connect service.
    pub running: &'a hephaestus_app::RunningHephaestus,
    /// Instance whose active revision is being advanced.
    pub instance_id: Uuid,
    /// Gateway whose exact mailbox publication identifies cooking events.
    pub gateway: &'a super::GatewayGoldenFixture,
    /// Revision active before this update sequence.
    pub current_revision_id: Uuid,
    /// Owner used by the mediator authorization policy.
    pub owner: Uuid,
    /// Creates a fresh assertion for each exact RPC audience.
    pub rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    /// Exact source rule identities active on this revision.
    pub brokered_rule_ids: BrokeredRuleIds,
    /// Bound for each lifecycle wait.
    pub timeout: Duration,
}

/// Stable IDs returned by the production `CreateUpdate` command.
// The type intentionally groups the durable IDs returned by one RPC;
// each field names a different persisted identity.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
pub struct UpdateIds {
    /// Durable update identity.
    pub update_id: Uuid,
    /// Candidate instance revision created by the command.
    pub candidate_revision_id: Uuid,
    /// Fresh model rule identity carried by the candidate revision.
    pub model_rule_id: Uuid,
    /// Fresh relay rule identity carried by the candidate revision.
    pub relay_rule_id: Uuid,
}

/// The two brokered rules carried by one immutable cooking revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokeredRuleIds {
    pub model: Uuid,
    pub relay: Uuid,
}

/// Published agents and explicit rule mappings used by one update sequence.
#[derive(Debug, Clone, Copy)]
pub struct CookingUpdateCandidates {
    pub migrate_release_agent_id: Uuid,
    pub rollback_release_agent_id: Uuid,
    pub abnormal_release_agent_id: Uuid,
    pub migrate_rule_ids: BrokeredRuleIds,
    pub rollback_rule_ids: BrokeredRuleIds,
    pub abnormal_rule_ids: BrokeredRuleIds,
}

impl BrokeredRuleIds {
    pub fn fresh() -> Self {
        Self {
            model: Uuid::new_v4(),
            relay: Uuid::new_v4(),
        }
    }

    fn copies_from(self, source: Self) -> Vec<BrokeredRuleCopy> {
        vec![
            BrokeredRuleCopy {
                source_rule_id: opaque(source.model).into(),
                candidate_rule_id: opaque(self.model).into(),
                ..Default::default()
            },
            BrokeredRuleCopy {
                source_rule_id: opaque(source.relay).into(),
                candidate_rule_id: opaque(self.relay).into(),
                ..Default::default()
            },
        ]
    }
}

/// Durable update lifecycle projection used in assertions and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateState {
    /// `agent_updates.state`.
    pub update: String,
    /// `agent_instances.state`.
    pub instance: String,
    /// Current durable run gate value.
    pub run_gate_open: bool,
    /// Active instance revision, if one is recorded.
    pub active_revision_id: Option<Uuid>,
    /// Candidate revision attached to this update.
    pub candidate_revision_id: Uuid,
}

type DeferredEventProjection = (
    String,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
);

fn deferred_event_is_terminal_success(
    projection: &DeferredEventProjection,
    candidate_revision_id: Uuid,
) -> bool {
    let (disposition, Some(_run_id), revision_id, run_state, outcome) = projection else {
        return false;
    };
    *revision_id == Some(candidate_revision_id)
        && disposition == "delivered"
        && run_state.as_deref() == Some("cleaned_up")
        && outcome.as_deref() == Some("succeeded")
}

/// IDs and active revision returned by the complete three-variant update
/// sequence.
#[derive(Debug, Clone, Copy)]
pub struct UpdateSequence {
    /// Migration update that activated the v2 candidate.
    pub migration: UpdateIds,
    /// Abnormal hook update recovered by operator rejection.
    pub abnormal: UpdateIds,
    /// The completed pre-rotation relay event whose old lease remains in history.
    pub relay_run_id: Uuid,
    /// Relay version rotation performed before candidate rule cloning.
    pub relay_rotation: super::cooking::CredentialRotation,
}

/// Secret versions observed across one held v1 request and its later dispatch.
#[derive(Debug, Clone, Copy)]
pub struct CredentialRotation {
    /// Version pinned by the in-flight v1 lease.
    pub pinned_version_id: Uuid,
    /// Version selected by the later dispatch after rotation commits.
    pub rotated_version_id: Uuid,
}

/// Admits an update only while a known v1 run is still active.  This is the
/// reusable production barrier: the SQL lifecycle check proves the run uses
/// the expected revision, then `CreateUpdate` must observe it during drain and
/// leave the gate closed until that run is cleaned up.
pub async fn begin_update_while_v1_active(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    active_run_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    assert_active_v1_run(context.pool, active_run_id, context.current_revision_id).await;
    let ids = create_update(context, candidate_release_agent_id, candidate_rule_ids).await;
    let state = load_state(context.pool, ids.update_id).await;
    assert!(!state.run_gate_open, "active v1 drain closes the run gate");
    assert_eq!(state.active_revision_id, Some(context.current_revision_id));
    assert_eq!(state.update, "draining");
    let run_state: String = sqlx::query_scalar("SELECT state FROM runs WHERE id = $1")
        .bind(active_run_id)
        .fetch_one(context.pool)
        .await
        .expect("active v1 run after update admission");
    assert!(
        [
            "queued",
            "leasing_volume",
            "provisioning",
            "starting",
            "running"
        ]
        .contains(&run_state.as_str()),
        "update must wait for the active v1 run to drain: {run_state}"
    );
    ids
}

/// Runs migration, deferred mailbox selection, explicit rollback, and
/// abnormal-hook recovery using three candidates from one published family.
/// The callback must submit a real gateway event and return its mailbox event
/// ID while the migration gate is closed; it must not wait for that run.
pub async fn exercise_update_sequence<F, Fut>(
    context: &CookingUpdateContext<'_>,
    active_v1_run_id: Uuid,
    candidates: CookingUpdateCandidates,
    relay_run_id: Uuid,
    relay_rotation: super::cooking::CredentialRotation,
    submit_deferred_event: F,
) -> UpdateSequence
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Uuid>,
{
    let migration = begin_update_while_v1_active(
        context,
        candidates.migrate_release_agent_id,
        active_v1_run_id,
        candidates.migrate_rule_ids,
    )
    .await;
    let deferred_event_id = submit_deferred_event().await;
    assert_event_waiting_behind_gate(context.pool, deferred_event_id).await;
    finish_compatible_update(context, migration).await;
    assert_deferred_event_uses_revision(
        context.pool,
        deferred_event_id,
        migration.candidate_revision_id,
        context.timeout,
    )
    .await;
    let migrated_context = CookingUpdateContext {
        pool: context.pool,
        running: context.running,
        instance_id: context.instance_id,
        gateway: context.gateway,
        current_revision_id: migration.candidate_revision_id,
        owner: context.owner,
        rpc_token: context.rpc_token,
        brokered_rule_ids: BrokeredRuleIds {
            model: migration.model_rule_id,
            relay: migration.relay_rule_id,
        },
        timeout: context.timeout,
    };
    exercise_explicit_rollback(
        &migrated_context,
        candidates.rollback_release_agent_id,
        candidates.rollback_rule_ids,
    )
    .await;
    let abnormal = exercise_abnormal_recovery(
        &migrated_context,
        candidates.abnormal_release_agent_id,
        candidates.abnormal_rule_ids,
    )
    .await;
    UpdateSequence {
        migration,
        abnormal,
        relay_run_id,
        relay_rotation,
    }
}

/// Runs the complete update sequence with a real active v1 request held at
/// the model boundary. Request 47 enters before `CreateUpdate`, request 48 is
/// accepted behind the closed gate, and releasing the model response lets the
/// normal v1 run clean up before the migration hook is admitted.
pub async fn exercise_barrier_update_sequence(
    context: &CookingUpdateContext<'_>,
    upstream: &super::BrokeredTlsUpstream,
    candidates: CookingUpdateCandidates,
) -> UpdateSequence {
    let public = std::env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = reqwest::Client::new();
    let held = super::cooking::send_update(&client, &url, 47, 1001, "stew").await;
    assert_eq!(held.status(), reqwest::StatusCode::OK);
    held.bytes().await.expect("held cooking acknowledgement");
    upstream.wait_update_v1_entered().await;
    let active = super::cooking::wait_for_active_event_run(
        context.pool,
        context.gateway.mailbox_id.as_uuid(),
        context.timeout,
        47,
    )
    .await;
    let rotation = rotate_model_credential_while_held(context, active.run_id.as_uuid()).await;
    // Event 46 is the completed relay fault/retry proof.  Keep its original
    // lease as the historical old-version assertion, while event 47 proves
    // that an in-flight v1 run has already issued its relay lease before the
    // source rotates and the candidate rules are cloned.
    let relay_run = super::cooking::wait_for_event_run(context.pool, context.gateway, 46).await;
    wait_for_active_brokered_lease(
        context.pool,
        active.run_id.as_uuid(),
        super::cooking::RELAY_RULE,
        context.timeout,
    )
    .await;
    let relay_rotation = super::cooking::rotate_brokered_credential(
        context.pool,
        UserId::from_uuid(context.owner),
        relay_run.run_id.as_uuid(),
        super::cooking::RELAY_RULE,
        super::cooking::RELAY_ROTATED_SENTINEL,
    )
    .await;
    let sequence = exercise_update_sequence(
        context,
        active.run_id.as_uuid(),
        candidates,
        relay_run.run_id.as_uuid(),
        relay_rotation,
        || async {
            let queued = super::cooking::send_update(&client, &url, 48, 1002, "curry").await;
            assert_eq!(queued.status(), reqwest::StatusCode::OK);
            queued
                .bytes()
                .await
                .expect("deferred cooking acknowledgement");
            let event_id = super::cooking::wait_for_event_id(
                context.pool,
                context.gateway.mailbox_id.as_uuid(),
                context.timeout,
                48,
            )
            .await;
            assert_event_waiting_behind_gate(context.pool, event_id).await;
            upstream.release_update_v1();
            event_id
        },
    )
    .await;
    let deferred_event_id = super::cooking::wait_for_event_id(
        context.pool,
        context.gateway.mailbox_id.as_uuid(),
        context.timeout,
        48,
    )
    .await;
    assert_rotated_model_lease(
        context.pool,
        active.run_id.as_uuid(),
        deferred_event_id,
        sequence.migration.model_rule_id,
        rotation,
    )
    .await;
    super::cooking::wait_for_event_run(context.pool, context.gateway, 47).await;
    sequence
}

async fn wait_for_active_brokered_lease(
    pool: &PgPool,
    run_id: Uuid,
    rule_id: Uuid,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*)
               FROM brokered_secret_lease_snapshots snapshot
               JOIN secret_leases lease ON lease.id = snapshot.lease_id
              WHERE snapshot.run_id = $1 AND snapshot.rule_id = $2
                AND lease.status = 'active' AND lease.expires_at > now()",
        )
        .bind(run_id)
        .bind(rule_id)
        .fetch_one(pool)
        .await
        .expect("active brokered lease before rotation");
        if count == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "held v1 relay lease was not issued before rotation"
        );
        sleep(Duration::from_millis(25)).await;
    }
}

/// Rotates the model secret after the held v1 run has acquired its lease.
/// The operation uses the same authenticated secret service boundary as the
/// product RPC and returns only opaque version identities.
pub async fn rotate_model_credential_while_held(
    context: &CookingUpdateContext<'_>,
    held_run_id: Uuid,
) -> CredentialRotation {
    let (secret_id, pinned_version_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT imported.secret_id, snapshot.secret_version_id
           FROM brokered_secret_lease_snapshots snapshot
           JOIN brokered_secret_rules rule ON rule.id = snapshot.rule_id
           JOIN agent_secret_bindings binding ON binding.id = snapshot.binding_id
           JOIN secret_imports imported ON imported.id = binding.import_id
          WHERE snapshot.run_id = $1 AND rule.id = $2",
    )
    .bind(held_run_id)
    .bind(super::cooking::MODEL_RULE)
    .fetch_one(context.pool)
    .await
    .expect("held v1 model lease snapshot");
    let owner = UserId::from_uuid(context.owner);
    let identity = AuthenticatedIdentity::new(
        owner,
        super::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let new_version = SecretVersionId::new();
    let service = SecretService::new(
        context.pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("cooking rotation key"),
        ),
        std::sync::Arc::new(PostgresMelangeAuthorizer),
    );
    service
        .rotate(
            &identity,
            RotateSecret {
                command_key: SecretCommandKey::derive(
                    "cooking-rotate-model",
                    &[new_version.as_uuid().as_bytes()],
                ),
                secret_id: SecretId::from_uuid(secret_id),
                expected_active_version_id: SecretVersionId::from_uuid(pinned_version_id),
                new_version_id: new_version,
                value: SecretValue::new(super::cooking::MODEL_ROTATED_SENTINEL)
                    .expect("rotated model sentinel"),
            },
        )
        .await
        .expect("rotate model credential while v1 lease is held");
    let active_version: Uuid =
        sqlx::query_scalar("SELECT active_version_id FROM secrets WHERE id = $1")
            .bind(secret_id)
            .fetch_one(context.pool)
            .await
            .expect("rotated model active version");
    assert_eq!(active_version, new_version.as_uuid());
    CredentialRotation {
        pinned_version_id,
        rotated_version_id: active_version,
    }
}

async fn assert_rotated_model_lease(
    pool: &PgPool,
    held_run_id: Uuid,
    deferred_event_id: Uuid,
    current_model_rule_id: Uuid,
    rotation: CredentialRotation,
) {
    let held_version: Uuid = sqlx::query_scalar(
        "SELECT secret_version_id FROM brokered_secret_lease_snapshots
          WHERE run_id = $1 AND rule_id = $2",
    )
    .bind(held_run_id)
    .bind(super::cooking::MODEL_RULE)
    .fetch_one(pool)
    .await
    .expect("held model lease version");
    assert_eq!(held_version, rotation.pinned_version_id);
    let later_version: Uuid = sqlx::query_scalar(
        "SELECT snapshot.secret_version_id
           FROM mailbox_delivery_attempts attempt
           JOIN brokered_secret_lease_snapshots snapshot
             ON snapshot.run_id = attempt.run_id AND snapshot.rule_id = $2
          WHERE attempt.event_id = $1
          ORDER BY attempt.attempt_number DESC
          LIMIT 1",
    )
    .bind(deferred_event_id)
    .bind(current_model_rule_id)
    .fetch_one(pool)
    .await
    .expect("later model lease version");
    assert_eq!(later_version, rotation.rotated_version_id);
    assert_ne!(held_version, later_version);
}

/// Completes a previously admitted compatible migration candidate.
pub async fn finish_compatible_update(context: &CookingUpdateContext<'_>, ids: UpdateIds) {
    wait_for_state(context, ids.update_id, "activated").await;
    let state = load_state(context.pool, ids.update_id).await;
    assert_eq!(state.instance, "active");
    assert!(state.run_gate_open);
    assert_eq!(state.active_revision_id, Some(ids.candidate_revision_id));
    assert_candidate_brokered_rules(context.pool, ids).await;
}

/// Confirms that the production update transaction carried both explicit
/// candidate rules and that its ordinary parameters select those same IDs.
/// This is intentionally a projection check: the persistence regression owns
/// the full canonical-hash/source-immutability proof.
async fn assert_candidate_brokered_rules(pool: &PgPool, ids: UpdateIds) {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT rule.id, binding.slot_key
           FROM brokered_secret_rules AS rule
           JOIN agent_secret_bindings AS binding
             ON binding.id = rule.binding_id
            AND binding.instance_revision_id = rule.instance_revision_id
          WHERE rule.instance_revision_id = $1
          ORDER BY binding.slot_key",
    )
    .bind(ids.candidate_revision_id)
    .fetch_all(pool)
    .await
    .expect("candidate brokered rule projection");
    assert_eq!(
        rows,
        vec![
            (ids.model_rule_id, String::from("model")),
            (ids.relay_rule_id, String::from("telegram_relay")),
        ],
        "candidate revision has exactly the declared model and relay rules"
    );
    let parameters: serde_json::Value =
        sqlx::query_scalar("SELECT parameters FROM agent_instance_revisions WHERE id = $1")
            .bind(ids.candidate_revision_id)
            .fetch_one(pool)
            .await
            .expect("candidate cooking parameters");
    let model_rule_id = ids.model_rule_id.to_string();
    let relay_rule_id = ids.relay_rule_id.to_string();
    assert_eq!(
        parameters["model_rule_id"].as_str(),
        Some(model_rule_id.as_str())
    );
    assert_eq!(
        parameters["relay_rule_id"].as_str(),
        Some(relay_rule_id.as_str())
    );
}

/// Drives a candidate whose update hook exits nonzero.  This is the explicit
/// agent rollback contract: the prior revision remains active and the update
/// is rejected without claiming to roll back agent-owned state.
pub async fn exercise_explicit_rollback(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let ids = create_update(context, candidate_release_agent_id, candidate_rule_ids).await;
    wait_for_state(context, ids.update_id, "rejected").await;
    let state = load_state(context.pool, ids.update_id).await;
    assert_eq!(state.instance, "update_rejected");
    assert!(state.run_gate_open);
    assert_eq!(state.active_revision_id, Some(context.current_revision_id));
    assert_update_event_order(context.pool, ids.update_id, "agent_update.rejected.v1").await;
    ids
}

/// Drives a signal-terminated or otherwise ambiguous hook to the
/// compatibility-unknown state, then performs the authorized reject action.
/// The assertion intentionally makes no claim about guest-owned rollback.
pub async fn exercise_abnormal_recovery(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let ids =
        exercise_abnormal_pending(context, candidate_release_agent_id, candidate_rule_ids).await;
    recover_update(context, ids.update_id, "RECOVERY_ACTION_REJECT").await;
    wait_for_state(context, ids.update_id, "rejected").await;
    let recovered = load_state(context.pool, ids.update_id).await;
    assert_eq!(recovered.instance, "update_rejected");
    assert!(recovered.run_gate_open);
    assert_eq!(
        recovered.active_revision_id,
        Some(context.current_revision_id)
    );
    assert_update_event_order(context.pool, ids.update_id, "agent_update.uncertain.v1").await;
    ids
}

/// Creates the abnormal update and leaves it in its paused compatibility
/// state for the browser recovery phase to resolve through the UI.
pub async fn exercise_abnormal_pending(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let ids = create_update(context, candidate_release_agent_id, candidate_rule_ids).await;
    wait_for_state(context, ids.update_id, "compatibility_unknown").await;
    let paused = load_state(context.pool, ids.update_id).await;
    assert_eq!(paused.instance, "paused_unknown_state");
    assert!(!paused.run_gate_open);
    assert_eq!(paused.active_revision_id, Some(context.current_revision_id));
    ids
}

/// Verifies that a mailbox event accepted behind the closed gate eventually
/// runs against the newly active revision.  The event ID must come from the
/// real gateway publication, so a hand-built deferred row cannot satisfy this
/// check.  Call [`assert_event_waiting_behind_gate`] before reopening the gate
/// when the pre-activation pending state also needs to be recorded.
pub async fn assert_deferred_event_uses_revision(
    pool: &PgPool,
    event_id: Uuid,
    candidate_revision_id: Uuid,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let row: Option<DeferredEventProjection> = sqlx::query_as(
            "SELECT delivery.disposition, attempt.run_id,
                    run.instance_revision_id, run.state, run.outcome
               FROM mailbox_deliveries delivery
               LEFT JOIN mailbox_delivery_attempts attempt
                 ON attempt.event_id = delivery.event_id
               LEFT JOIN runs run ON run.id = attempt.run_id
              WHERE delivery.event_id = $1
              ORDER BY attempt.attempt_number DESC NULLS LAST
              LIMIT 1",
        )
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .expect("deferred cooking event projection");
        if let Some((disposition, Some(run_id), revision_id, run_state, outcome)) = row {
            assert_eq!(revision_id, Some(candidate_revision_id));
            assert_ne!(run_id, Uuid::nil());
            if run_state.as_deref() == Some("cleaned_up") {
                assert_eq!(
                    outcome.as_deref(),
                    Some("succeeded"),
                    "deferred mailbox run reached terminal failure: disposition={disposition}"
                );
                assert_eq!(
                    disposition, "delivered",
                    "successful deferred mailbox run must have a terminal delivered disposition"
                );
                return;
            }
            assert!(
                ["pending", "eligible", "leased", "running", "retryable"]
                    .contains(&disposition.as_str()),
                "deferred mailbox event has unexpected nonterminal disposition {disposition}"
            );
        }
        assert!(
            Instant::now() < deadline,
            "deferred event did not materialize against candidate revision"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// Asserts the pre-activation half of the mailbox barrier.  A normal cooking
/// event accepted while the instance gate is closed has a durable delivery,
/// but no delivery attempt or run yet.
pub async fn assert_event_waiting_behind_gate(pool: &PgPool, event_id: Uuid) {
    let row: (String, i64) = sqlx::query_as(
        "SELECT delivery.disposition,
                (SELECT count(*) FROM mailbox_delivery_attempts attempt
                  WHERE attempt.event_id = delivery.event_id)
           FROM mailbox_deliveries delivery
          WHERE delivery.event_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("mailbox event barrier projection");
    assert!(
        ["pending", "eligible", "retryable"].contains(&row.0.as_str()),
        "event accepted behind a closed gate must remain dispatchable: {}",
        row.0
    );
    assert_eq!(row.1, 0, "gated event must not start a pre-update run");
}

/// Checks the committed `SQLite` migration in a host-visible state-volume
/// snapshot.  The path is supplied by the fixture because libkrun and local
/// backends expose different volume roots.  Python's `SQLite` client reads the
/// schema and rows through `SQLite` itself, including any associated WAL, so a
/// string search over raw database bytes cannot satisfy this assertion.
pub fn assert_migrated_sqlite(path: &Path, expected_recipe_count: usize) {
    let script = r#"
import sqlite3, sys
path, expected = sys.argv[1], int(sys.argv[2])
db = sqlite3.connect('file:' + path + '?mode=ro', uri=True)
version = db.execute('SELECT version FROM schema_meta').fetchone()[0]
assert version == 2, version
columns = {row[1] for row in db.execute('PRAGMA table_info(recipes)')}
assert 'recipe_summary' in columns, columns
indexes = {row[1] for row in db.execute('PRAGMA index_list(recipes)')}
assert 'recipes_summary' in indexes, indexes
recipes = db.execute('SELECT count(*) FROM recipes').fetchone()[0]
processed = db.execute('SELECT count(*) FROM processed_updates').fetchone()[0]
assert recipes == expected, (recipes, expected)
assert processed == expected, (processed, expected)
assert db.execute("SELECT count(*) FROM recipes WHERE context = ''").fetchone()[0] == 0
assert db.execute("SELECT count(*) FROM recipes WHERE recipe_summary = ''").fetchone()[0] == 0
if expected == 9:
    expected_ids = {f'recipe-{update_id}' for update_id in range(42, 51)}
    recipe_rows = db.execute(
        "SELECT recipe_id, publication_outcome, model_response IS NOT NULL, "
        "relay_outcome IS NOT NULL FROM recipes"
    ).fetchall()
    assert {row[0] for row in recipe_rows} == expected_ids
    for recipe_id, publication, has_model, has_relay in recipe_rows:
        if recipe_id == 'recipe-50':
            assert publication == 'pending'
            assert has_model
            assert not has_relay
        else:
            assert publication == 'proposal_ready'
            assert has_model
            assert has_relay
    dispositions = dict(db.execute(
        "SELECT update_id, disposition FROM processed_updates"
    ).fetchall())
    assert set(dispositions) == set(range(42, 51))
    assert all(dispositions[update_id] == 'completed' for update_id in range(42, 50))
    assert dispositions[50] == 'pending'
db.close()
"#;
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(path)
        .arg(expected_recipe_count.to_string())
        .output()
        .expect("run SQLite migration inspection");
    assert!(
        output.status.success(),
        "SQLite migration/schema conservation inspection failed"
    );
}

/// Extracts the committed state database from a detached ext4 volume and
/// validates it through `SQLite`. The volume must be detached before this is
/// called so `SQLite`'s WAL and main database are a consistent snapshot.
pub fn assert_migrated_sqlite_disk(disk: &Path, expected_recipe_count: usize) {
    assert!(
        disk.is_file(),
        "state-volume image is required for SQLite inspection"
    );
    let debugfs = std::process::Command::new("debugfs")
        .arg("-V")
        .output()
        .expect("debugfs is required for state-volume inspection");
    assert!(
        debugfs.status.success(),
        "debugfs prerequisite failed: {}",
        String::from_utf8_lossy(&debugfs.stderr)
    );
    let temporary = tempfile::tempdir().expect("create SQLite snapshot directory");
    let snapshot = temporary.path().join("cooking.sqlite3");
    let dump = |guest_path: &str, destination: &Path, required: bool| {
        let destination = destination.to_str().expect("SQLite snapshot path UTF-8");
        let output = std::process::Command::new("debugfs")
            .args(["-R", &format!("dump {guest_path} {destination}")])
            .arg(disk)
            .output()
            .expect("extract SQLite snapshot from state volume");
        if required {
            assert!(
                output.status.success(),
                "state volume SQLite extraction failed"
            );
            assert!(
                destination_path(destination),
                "SQLite snapshot is not a file"
            );
        }
    };
    dump("/cooking.sqlite3", &snapshot, true);
    // SQLite WAL mode may leave committed pages outside the main database.
    // Preserve both sidecars when present; the required main file check above
    // also prevents debugfs's zero exit status from hiding a missing path.
    dump(
        "/cooking.sqlite3-wal",
        &snapshot.with_extension("sqlite3-wal"),
        false,
    );
    dump(
        "/cooking.sqlite3-shm",
        &snapshot.with_extension("sqlite3-shm"),
        false,
    );
    super::cooking_confinement::assert_state_snapshot_has_no_credentials(temporary.path());
    assert_migrated_sqlite(&snapshot, expected_recipe_count);
}

fn destination_path(value: &str) -> bool {
    Path::new(value).is_file()
}

async fn create_update(
    context: &CookingUpdateContext<'_>,
    candidate_release_agent_id: Uuid,
    candidate_rule_ids: BrokeredRuleIds,
) -> UpdateIds {
    let audience = "/hephaestus.instance.v1.AgentInstanceService/CreateUpdate";
    let client = instance_client(context.running, context.rpc_token, audience);
    let response = client
        .create_update(CreateUpdateRequest {
            context: request_context("cooking-update").into(),
            instance_id: opaque(context.instance_id).into(),
            expected_revision_id: opaque(context.current_revision_id).into(),
            candidate_release_agent_id: opaque(candidate_release_agent_id).into(),
            parameters: super::cooking_builds::cooking_agent_parameters_for_rules(
                candidate_rule_ids.model,
                candidate_rule_ids.relay,
            ),
            brokered_rule_copies: candidate_rule_ids.copies_from(context.brokered_rule_ids),
            selected_policy: RuntimePolicy {
                vcpus: 1,
                memory_mib: 256,
                network: NetworkPolicy::BrokerOnly.into(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .expect("CreateUpdate request");
    let body = response.into_owned();
    UpdateIds {
        update_id: body
            .update_id
            .into_option()
            .expect("CreateUpdate update ID")
            .value
            .parse()
            .expect("CreateUpdate update ID UUID"),
        candidate_revision_id: body
            .candidate_revision_id
            .into_option()
            .expect("CreateUpdate candidate revision ID")
            .value
            .parse()
            .expect("CreateUpdate candidate revision UUID"),
        model_rule_id: candidate_rule_ids.model,
        relay_rule_id: candidate_rule_ids.relay,
    }
}

async fn assert_active_v1_run(pool: &PgPool, run_id: Uuid, revision_id: Uuid) {
    let row: (String, Uuid) = sqlx::query_as(
        "SELECT state, instance_revision_id FROM runs
          WHERE id = $1 AND run_kind = 'normal'",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("active v1 run barrier");
    assert_eq!(row.1, revision_id, "barrier run must use the v1 revision");
    assert!(
        [
            "queued",
            "leasing_volume",
            "provisioning",
            "starting",
            "running"
        ]
        .contains(&row.0.as_str()),
        "barrier run is already terminal: {}",
        row.0
    );
}

async fn recover_update(context: &CookingUpdateContext<'_>, update_id: Uuid, action: &str) {
    let audience = "/hephaestus.instance.v1.AgentInstanceService/RecoverUpdate";
    let action = match action {
        "RECOVERY_ACTION_RETRY" => RecoveryAction::Retry,
        "RECOVERY_ACTION_REJECT" => RecoveryAction::Reject,
        "RECOVERY_ACTION_RESUME" => RecoveryAction::Resume,
        _ => panic!("unsupported update recovery action {action}"),
    };
    instance_client(context.running, context.rpc_token, audience)
        .recover_update(RecoverUpdateRequest {
            context: request_context("cooking-recover-update").into(),
            update_id: opaque(update_id).into(),
            action: action.into(),
            ..Default::default()
        })
        .await
        .expect("RecoverUpdate request");
}

/// Proves fast-hook lifecycle ordering without sampling the gate after the
/// RPC returns. A completed rollback or uncertainty transition may legitimately
/// reopen the gate before the caller can inspect it.
async fn assert_update_event_order(pool: &PgPool, update_id: Uuid, terminal_event: &str) {
    let rows: Vec<(String, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT event_type, min(occurred_at)
           FROM outbox
          WHERE aggregate_id = $1
            AND event_type IN (
                'agent_update.requested.v1',
                'agent_update.hook_started.v1',
                'agent_update.rejected.v1',
                'agent_update.uncertain.v1'
            )
          GROUP BY event_type
          ORDER BY min(occurred_at)",
    )
    .bind(update_id)
    .fetch_all(pool)
    .await
    .expect("durable update lifecycle events");
    let timestamps = rows
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let requested = timestamps
        .get("agent_update.requested.v1")
        .copied()
        .expect("durable update requested event");
    let started = timestamps
        .get("agent_update.hook_started.v1")
        .copied()
        .expect("durable update hook started event");
    let terminal = timestamps
        .get(terminal_event)
        .copied()
        .expect("durable update terminal event");
    assert!(requested <= started && started <= terminal);
}

async fn wait_for_state(context: &CookingUpdateContext<'_>, update_id: Uuid, wanted: &str) {
    let deadline = Instant::now() + context.timeout;
    loop {
        if load_state(context.pool, update_id).await.update == wanted {
            return;
        }
        if Instant::now() >= deadline {
            let lifecycle = query_timeout_lifecycle(context.pool, update_id).await;
            let hook = match lifecycle.as_ref().and_then(|row| row.5) {
                Some(run_id) => query_timeout_hook(context.pool, run_id).await,
                None => None,
            };
            panic!(
                "update {update_id} did not reach state {wanted}; lifecycle={}; hook={}",
                format_timeout_lifecycle(lifecycle.as_ref()),
                format_timeout_hook(hook.as_ref())
            );
        }
        sleep(Duration::from_millis(100)).await;
    }
}

type TimeoutLifecycle = (
    String,
    String,
    bool,
    Option<Uuid>,
    Uuid,
    Option<Uuid>,
    Option<Uuid>,
    String,
);

type TimeoutHook = (String, Option<String>, Option<i32>, Option<i32>);

async fn query_timeout_lifecycle(pool: &PgPool, update_id: Uuid) -> Option<TimeoutLifecycle> {
    // Diagnostics are deliberately best-effort. A database timeout or
    // migration mismatch must not replace the original state timeout.
    timeout(
        Duration::from_secs(2),
        sqlx::query_as::<_, TimeoutLifecycle>(
            "SELECT update_record.state, instance.state,
                    instance.run_gate_open, instance.active_revision_id,
                    update_record.candidate_revision_id, update_record.hook_run_id,
                    candidate.release_agent_id,
                    CASE
                      WHEN COALESCE((candidate_agent.update_hook -> 'arguments') ? '--abnormal-fixture', false)
                        THEN 'abnormal'
                      WHEN COALESCE((candidate_agent.update_hook -> 'arguments') ? '--rollback-fixture', false)
                        THEN 'rollback'
                      WHEN COALESCE((candidate_agent.update_hook -> 'arguments') ? '--migrate', false)
                        THEN 'normal'
                      ELSE 'unknown'
                    END
               FROM agent_updates AS update_record
               JOIN agent_instances AS instance
                 ON instance.id = update_record.instance_id
               LEFT JOIN agent_instance_revisions AS candidate
                 ON candidate.id = update_record.candidate_revision_id
               LEFT JOIN release_agents AS candidate_agent
                 ON candidate_agent.id = candidate.release_agent_id
              WHERE update_record.id = $1",
        )
        .bind(update_id)
        .fetch_optional(pool),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
}

async fn query_timeout_hook(pool: &PgPool, run_id: Uuid) -> Option<TimeoutHook> {
    timeout(
        Duration::from_secs(2),
        sqlx::query_as::<_, TimeoutHook>(
            "SELECT state, outcome, exit_code, exit_signal
               FROM runs
              WHERE id = $1",
        )
        .bind(run_id)
        .fetch_optional(pool),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
}

fn format_timeout_lifecycle(row: Option<&TimeoutLifecycle>) -> String {
    row.map_or_else(
        || String::from("unavailable"),
        |row| {
            format!(
                "update_state={};instance_state={};gate_open={};active_revision_id={:?};candidate_revision_id={};hook_run_id={:?};release_agent_id={:?};hook_mode={}",
                safe_update_state(&row.0),
                safe_instance_state(&row.1),
                row.2,
                row.3,
                row.4,
                row.5,
                row.6,
                row.7,
            )
        },
    )
}

fn format_timeout_hook(row: Option<&TimeoutHook>) -> String {
    row.map_or_else(
        || String::from("unavailable"),
        |row| {
            format!(
                "state={};outcome={};exit_code={:?};exit_signal={:?}",
                safe_run_state(&row.0),
                safe_run_outcome(row.1.as_deref()),
                row.2,
                row.3
            )
        },
    )
}

fn safe_update_state(value: &str) -> &'static str {
    match value {
        "candidate" => "candidate",
        "draining" => "draining",
        "hook_running" => "hook_running",
        "hook_committed" => "hook_committed",
        "activated" => "activated",
        "rejected" => "rejected",
        "compatibility_unknown" => "compatibility_unknown",
        "activation_recovery" => "activation_recovery",
        _ => "unknown",
    }
}

fn safe_instance_state(value: &str) -> &'static str {
    match value {
        "active" => "active",
        "disabled" => "disabled",
        "update_draining" => "update_draining",
        "updating" => "updating",
        "update_rejected" => "update_rejected",
        "paused_unknown_state" => "paused_unknown_state",
        "paused_activation_recovery" => "paused_activation_recovery",
        "recovering" => "recovering",
        "removed" => "removed",
        _ => "unknown",
    }
}

fn safe_run_state(value: &str) -> &'static str {
    match value {
        "queued" => "queued",
        "leasing_volume" => "leasing_volume",
        "provisioning" => "provisioning",
        "starting" => "starting",
        "running" => "running",
        "succeeded" => "succeeded",
        "failed" => "failed",
        "cancelled" => "cancelled",
        "cleaning_up" => "cleaning_up",
        "cleaned_up" => "cleaned_up",
        _ => "unknown",
    }
}

fn safe_run_outcome(value: Option<&str>) -> &'static str {
    match value {
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some(_) => "unknown",
        None => "none",
    }
}

async fn load_state(pool: &PgPool, update_id: Uuid) -> UpdateState {
    let row: (String, String, bool, Option<Uuid>, Uuid) = sqlx::query_as(
        "SELECT update.state, instance.state, instance.run_gate_open,
                instance.active_revision_id, update.candidate_revision_id
           FROM agent_updates update
           JOIN agent_instances instance ON instance.id = update.instance_id
          WHERE update.id = $1",
    )
    .bind(update_id)
    .fetch_one(pool)
    .await
    .expect("update lifecycle projection");
    UpdateState {
        update: row.0,
        instance: row.1,
        run_gate_open: row.2,
        active_revision_id: row.3,
        candidate_revision_id: row.4,
    }
}

fn instance_client(
    running: &hephaestus_app::RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> AgentInstanceServiceClient<connectrpc::client::HttpClient> {
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("instance RPC URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))
                .expect("instance RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    AgentInstanceServiceClient::new(connectrpc::client::HttpClient::plaintext(), config)
}

fn request_context(operation: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod deferred_event_tests {
    use super::{DeferredEventProjection, deferred_event_is_terminal_success};
    use uuid::Uuid;

    #[test]
    fn materialized_run_is_not_complete_while_delivery_is_eligible() {
        let candidate_revision_id = Uuid::new_v4();
        let projection: DeferredEventProjection = (
            String::from("eligible"),
            Some(Uuid::new_v4()),
            Some(candidate_revision_id),
            Some(String::from("queued")),
            None,
        );

        assert!(!deferred_event_is_terminal_success(
            &projection,
            candidate_revision_id
        ));
    }

    #[test]
    fn only_delivered_cleaned_success_is_complete() {
        let candidate_revision_id = Uuid::new_v4();
        let projection: DeferredEventProjection = (
            String::from("delivered"),
            Some(Uuid::new_v4()),
            Some(candidate_revision_id),
            Some(String::from("cleaned_up")),
            Some(String::from("succeeded")),
        );

        assert!(deferred_event_is_terminal_success(
            &projection,
            candidate_revision_id
        ));
    }
}
