use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use sqlx::PgPool;
use std::time::Duration;
use vm_trait::VmId;

use super::{
    fixture::{Role, role_pool, scope},
    seed,
};

pub async fn close_serializes_before_new_creation(pool: &PgPool) {
    let command = seed::normal(pool).await;
    let worker = role_pool(pool, Role::Worker).await;
    let repository = PgRunRepository::new(worker.clone()).with_vm_owner_scope(scope());
    let mut closing = worker.begin().await.unwrap();
    sqlx::query("UPDATE agent_instances SET run_gate_open=false WHERE id=$1")
        .bind(command.instance_id.as_uuid())
        .execute(&mut *closing)
        .await
        .unwrap();
    let mut creation = tokio::spawn({
        let command = command.clone();
        async move {
            repository
                .create_run_with_vm_plan(&command, &scope(), &VmId(command.run_id.to_string()))
                .await
        }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut creation)
            .await
            .is_err(),
        "creation must wait for consumer closure transaction"
    );
    closing.commit().await.unwrap();
    let denied = (&mut creation).await.unwrap();
    drop(creation);
    assert!(denied.is_err());
    let empty: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM runs WHERE id=$1) AND NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=$2)")
        .bind(command.run_id.as_uuid()).bind(command.command_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert!(empty);
    worker.close().await;
}
