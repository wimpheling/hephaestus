//! Durable ID selection for the cooking VM specification contracts.
//!
//! The observer records every successful real-provider provision, but a kind
//! census alone can accidentally validate an unrelated VM.  This module
//! selects the exact runs, gateway invocations, and build requests produced by
//! the cooking fixture before invoking the redacted per-kind assertions.

use super::vm_observer::VmSpecObserver;
use sqlx::PgPool;
use uuid::Uuid;

/// Durable IDs that identify the cooking operations whose VMs must be checked.
// The suffixes preserve the domain of each identifier at this boundary.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct CookingVmContractIds {
    /// Mailbox used by the canonical event 42/43 positive controls.
    pub(crate) canonical_mailbox_id: Uuid,
    /// Mailbox used by the signal-9 crash/retry events 51..55.
    pub(crate) crash_mailbox_id: Uuid,
    /// Run ID returned by the adversarial destination denial probe.
    pub(crate) adversarial_run_id: Uuid,
    /// Build request for the released gateway.
    pub(crate) gateway_build_request_id: Uuid,
    /// Build request for the released ordinary agent.
    pub(crate) agent_build_request_id: Uuid,
    /// Build request for the transformed crash agent.
    pub(crate) crash_agent_build_request_id: Uuid,
}

/// Selects durable run and invocation IDs and checks their complete VM specs.
pub(crate) async fn assert_cooking_vm_contracts(
    pool: &PgPool,
    observer: &VmSpecObserver,
    ids: CookingVmContractIds,
) {
    observer
        .assert_required_kinds(&["agent", "gateway", "build"])
        .expect("observe cooking agent, gateway, and build VM specifications");

    let canonical_keys = [
        String::from("telegram-update-42"),
        String::from("telegram-update-43"),
    ];
    let crash_keys = (51..=55)
        .map(|update_id| format!("telegram-update-{update_id}"))
        .collect::<Vec<_>>();

    let canonical_agent_ids =
        run_ids_for_events(pool, ids.canonical_mailbox_id, &canonical_keys).await;
    assert_eq!(
        canonical_agent_ids.len(),
        2,
        "cooking positive controls must select exactly two agent runs"
    );
    let crash_agent_ids = run_ids_for_events(pool, ids.crash_mailbox_id, &crash_keys).await;
    assert_eq!(
        crash_agent_ids.len(),
        10,
        "each of the five crash events must select its failed and recovered agent runs"
    );

    let adversarial_agent_ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM runs WHERE id = $1")
        .bind(ids.adversarial_run_id)
        .fetch_all(pool)
        .await
        .expect("select adversarial cooking agent run");
    assert_eq!(
        adversarial_agent_ids,
        vec![ids.adversarial_run_id],
        "adversarial denial must retain exactly one selected agent run"
    );

    let mut agent_ids = canonical_agent_ids;
    agent_ids.extend(crash_agent_ids);
    agent_ids.push(ids.adversarial_run_id);
    observer
        .assert_agent_contract(
            &agent_ids
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        )
        .expect("selected cooking agent VM specifications satisfy the contract");

    let canonical_gateway_ids =
        gateway_invocation_ids(pool, ids.canonical_mailbox_id, &canonical_keys).await;
    assert_eq!(
        canonical_gateway_ids.len(),
        2,
        "cooking positive controls must select exactly two gateway invocations"
    );
    let crash_gateway_ids = gateway_invocation_ids(pool, ids.crash_mailbox_id, &crash_keys).await;
    assert_eq!(
        crash_gateway_ids.len(),
        5,
        "each crash event must select one accepted gateway invocation"
    );
    let mut gateway_ids = canonical_gateway_ids;
    gateway_ids.extend(crash_gateway_ids);
    observer
        .assert_gateway_contract(
            &gateway_ids
                .into_iter()
                .map(|id| format!("gateway-{id}"))
                .collect::<Vec<_>>(),
        )
        .expect("selected cooking gateway VM specifications satisfy the contract");

    let build_ids = [
        ids.gateway_build_request_id,
        ids.agent_build_request_id,
        ids.crash_agent_build_request_id,
    ]
    .into_iter()
    .map(|id| format!("build-{id}"))
    .collect::<Vec<_>>();
    observer
        .assert_build_contract(&build_ids)
        .expect("selected cooking build VM specifications satisfy the contract");
}

async fn run_ids_for_events(pool: &PgPool, mailbox_id: Uuid, keys: &[String]) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT DISTINCT run.id
           FROM mailbox_events AS event
           JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = event.id
           JOIN runs AS run ON run.id = attempt.run_id
          WHERE event.mailbox_id = $1
            AND event.deduplication_key = ANY($2::text[])
          ORDER BY run.id",
    )
    .bind(mailbox_id)
    .bind(keys)
    .fetch_all(pool)
    .await
    .expect("select cooking agent run IDs from mailbox events")
}

async fn gateway_invocation_ids(pool: &PgPool, mailbox_id: Uuid, keys: &[String]) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT DISTINCT publication.invocation_id
           FROM gateway_mailbox_publications AS publication
          WHERE publication.mailbox_id = $1
            AND publication.deduplication_key = ANY($2::text[])
            AND publication.outcome = 'accepted'
          ORDER BY publication.invocation_id",
    )
    .bind(mailbox_id)
    .bind(keys)
    .fetch_all(pool)
    .await
    .expect("select cooking gateway invocation IDs from publications")
}
