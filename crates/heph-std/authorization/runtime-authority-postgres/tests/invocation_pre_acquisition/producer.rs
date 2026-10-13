use super::fixture::{Fixture, code};
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use run_domain::{RunKind, RunState};
use run_orchestrator::RunRepository;
use runtime_types::RunId;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn genuine_missing_consumption_and_fabricated_labels_do_not_authorize(pool: PgPool) {
    super::schema::verify_inputs();
    let birth = Box::pin(crate::fixture::Fixture::new(&pool, 0)).await;
    super::schema::apply(&pool).await;
    let before = birth.rows().await;
    let facts:(String,i64,bool)=sqlx::query_as("SELECT state,(SELECT count(*) FROM run_invocation_consumptions WHERE run_id=r.id),inspect_qualified_invocation117(r.id) FROM runs r WHERE id=$1")
        .bind(birth.start.run_id.as_uuid()).fetch_one(&birth.birth.worker).await.unwrap();
    assert_eq!(facts, ("queued".to_owned(), 0, false));
    // Actual117 protects typed hydration; no synthetic unconsumed Run is built.
    assert!(birth.repository().get(birth.start.run_id).await.is_err());
    let eligible:bool=sqlx::query_scalar("SELECT verify_invocation119_pre_acquisition(r.id,$2,r.instance_id,r.instance_revision_id,r.release_id,r.release_agent_id,r.command_id,r.requires_state,r.vm_id,r.state_version,$3) FROM runs r WHERE id=$1")
        .bind(birth.start.run_id.as_uuid()).bind(serde_json::to_vec(&birth.config.canonical_json()).unwrap())
        .bind(AUTHORIZATION_MODEL_VERSION).fetch_one(&birth.birth.worker).await.unwrap();
    assert!(!eligible);
    let error = sqlx::query(
        "UPDATE runs SET state='leasing_volume',state_version=state_version+1 WHERE id=$1",
    )
    .bind(birth.start.run_id.as_uuid())
    .execute(&birth.birth.worker)
    .await
    .unwrap_err();
    //119-A rejects progress before the required genuine117 consumption.
    assert_eq!(code(&error).as_deref(), Some("P1190"));
    assert_eq!(birth.rows().await, before);
    let f = Box::pin(Fixture::consumed(birth)).await;
    f.allowed().await;
    let mut unknown = f.run.clone();
    unknown.id = RunId::new();
    unknown.vm_id = Some(unknown.id.to_string());
    f.denied(&unknown).await;
    assert!(
        !f.raw(
            &f.birth.birth.worker,
            &unknown,
            &f.config(),
            AUTHORIZATION_MODEL_VERSION
        )
        .await
        .unwrap()
    );
    for (kind, state) in [
        (RunKind::Normal, RunState::LeasingVolume),
        (RunKind::Invocation, RunState::Queued),
    ] {
        let mut claimed = f.run.clone();
        claimed.kind = kind;
        claimed.state = state;
        f.denied(&claimed).await;
    }
}
