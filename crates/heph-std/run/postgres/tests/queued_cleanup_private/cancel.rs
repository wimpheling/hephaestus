//! Exact typed Cancel negatives and schema-valid wrong/pending transport history.
use super::{
    admission,
    observations::{code, held, rows},
    queued_fixture::Queued,
};
use run_orchestrator::RunRepository;

pub async fn before_delivery(h: &Queued) {
    let before = rows(&h.fixture.pool).await;
    for field in 0..3 {
        let mut wrong = h.cancel.clone();
        match field {
            0 => wrong.command_id = runtime_types::CommandId::new(),
            1 => wrong.run_id = runtime_types::RunId::new(),
            _ => wrong.reason = "foreign reason".into(),
        }
        let error = h.repository().request_cancel(&wrong).await.unwrap_err();
        if field == 1 {
            assert!(matches!(
                error,
                run_orchestrator::RepositoryError::NotFound(id) if id==wrong.run_id
            ));
        } else {
            held(&error);
        }
        assert_eq!(rows(&h.fixture.pool).await, before);
    }
    for pending in [true, false] {
        let mut tx = h.fixture.pool.begin().await.unwrap();
        let mut payload = serde_json::to_value(&h.cancel).unwrap();
        if !pending {
            payload["reason"] = serde_json::json!("foreign reason");
        }
        sqlx::query(
            "INSERT INTO command_inbox(command_id,command_type,payload) VALUES($1,'cancel_run',$2)",
        )
        .bind(h.cancel.command_id.as_uuid())
        .bind(payload)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await
            .unwrap();
        let error = admission::cancel_lock(&mut tx, h).await.unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("P1180"));
        tx.rollback().await.unwrap();
        assert_eq!(rows(&h.fixture.pool).await, before);
    }
}
pub async fn after_delivery(h: &Queued) {
    let before = rows(&h.fixture.pool).await;
    for wrong in [true, false] {
        let mut tx = h.fixture.pool.begin().await.unwrap();
        let query = if wrong {
            sqlx::query(
                "UPDATE command_inbox SET payload=jsonb_set(payload,'{reason}','\"foreign reason\"') WHERE command_id=$1",
            )
        } else {
            sqlx::query("UPDATE command_inbox SET processed_at=NULL WHERE command_id=$1")
        };
        assert_eq!(
            query
                .bind(h.cancel.command_id.as_uuid())
                .execute(&mut *tx)
                .await
                .unwrap()
                .rows_affected(),
            1
        );
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await
            .unwrap();
        let error = sqlx::query("SELECT * FROM load_invocation119_cleanup_only($1,$2,$3,$4,$5,$6)")
            .bind(h.request.run_id().as_uuid())
            .bind(h.request.instance_id().as_uuid())
            .bind(admission::original(h))
            .bind(h.fixture.scope.provider_namespace())
            .bind(h.fixture.scope.host_id())
            .bind(serde_json::to_vec(&h.fixture.config.canonical_json()).unwrap())
            .fetch_all(&mut *tx)
            .await
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("P1180"));
        tx.rollback().await.unwrap();
        assert_eq!(rows(&h.fixture.pool).await, before);
    }
}
