//! Rolled-back negative transport corruption; never accepted execution history.
use super::wake_fixture::Wake;

pub async fn assert_negatives(h: &Wake) {
    let stable = super::observations::rows(&h.fixture.pool).await;
    for query in [
        "DELETE FROM outbox WHERE aggregate_id=$1 AND event_type='run.cancel_requested.v1'",
        "UPDATE outbox SET payload=jsonb_set(payload,'{reason}','\"substituted reason\"') WHERE aggregate_id=$1 AND event_type='run.cancel_requested.v1'",
        "INSERT INTO outbox (id,aggregate_type,aggregate_id,subject,event_type,payload,occurred_at,published_at,attempts,last_error) SELECT gen_random_uuid(),o.aggregate_type,o.aggregate_id,o.subject,o.event_type,o.payload,o.occurred_at,o.published_at,o.attempts,o.last_error FROM outbox o WHERE aggregate_id=$1 AND event_type='run.cancel_requested.v1'",
        "UPDATE command_inbox SET processed_at=NULL WHERE command_type='cancel_run' AND payload->>'run_id'=$1::text",
        "UPDATE command_inbox SET payload=jsonb_set(payload,'{reason}','\"substituted reason\"') WHERE command_type='cancel_run' AND payload->>'run_id'=$1::text",
    ] {
        let mut tx = h.fixture.pool.begin().await.unwrap();
        // Preserve all constraints. Privileged fixture mutation stays uncommitted;
        // the actual worker facade must reject that contradictory snapshot.
        assert!(
            sqlx::query(query)
                .bind(h.request.run_id().as_uuid())
                .execute(&mut *tx)
                .await
                .unwrap()
                .rows_affected()
                > 0
        );
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await
            .unwrap();
        let pins = h.request.original();
        let original = serde_json::json!({"drain_attempt":pins.drain_attempt,"drain_command":pins.drain_command,"removal_id":pins.removal_id,"command_key":pins.command_key,"input_hash":pins.input_hash,"expected_version":pins.expected_version,"creator":pins.creator,"request":pins.request});
        let error = sqlx::query("SELECT * FROM load_run_cleanup_wake118($1,$2,$3,$4,$5,$6)")
            .bind(h.request.run_id().as_uuid())
            .bind(h.request.instance_id().as_uuid())
            .bind(original)
            .bind(h.fixture.scope.provider_namespace())
            .bind(h.fixture.scope.host_id())
            .bind(serde_json::to_vec(&h.fixture.config.canonical_json()).unwrap())
            .fetch_all(&mut *tx)
            .await
            .unwrap_err();
        assert_eq!(
            super::observations::code(&error).as_deref(),
            Some("P1180"),
            "{query}: {error:?}"
        );
        tx.rollback().await.unwrap();
        assert_eq!(super::observations::rows(&h.fixture.pool).await, stable);
    }
}
