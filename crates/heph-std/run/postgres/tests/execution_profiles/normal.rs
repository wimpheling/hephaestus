use run_domain::FreshRunExecutionProfile;
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use sqlx::PgPool;
use vm_trait::VmId;

use super::{
    fixture::{Role, role_pool, scope},
    profile_support,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL"]
async fn default_constructor_preserves_public107_sql_and_exact_replay(pool: PgPool) {
    super::fixture::checkpoint(107).run(&pool).await.unwrap();
    let command = profile_support::git(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    let repository = PgRunRepository::new(worker.clone());
    assert!(repository.create_run(&command).await.unwrap().created);
    assert!(!repository.create_run(&command).await.unwrap().created);
    let mut changed = command.clone();
    changed.requires_state = !command.requires_state;
    assert!(repository.create_run(&changed).await.is_err());
    let missing: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='runs' AND column_name='execution_profile')")
        .fetch_one(&pool).await.unwrap();
    assert!(missing);
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn explicit_legacy_git_records_profile_and_never_promotes_on_replay(pool: PgPool) {
    profile_support::profile_schema(&pool).await;
    let command = profile_support::git(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    let repository = PgRunRepository::new(worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar);
    assert!(repository.create_run(&command).await.unwrap().created);
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("legacy_scalar".into(), false, false)
    );
    assert!(!repository.create_run(&command).await.unwrap().created);
    let owned = PgRunRepository::new(worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope());
    assert!(
        owned
            .create_run_with_vm_plan(&command, &scope(), &VmId(command.run_id.to_string()))
            .await
            .is_err()
    );
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("legacy_scalar".into(), false, false)
    );
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn owned_normal_requires_explicit_profile_scope_and_accepts_update_rejected(pool: PgPool) {
    profile_support::profile_schema(&pool).await;
    let command = profile_support::git(&pool).await;
    sqlx::query("UPDATE agent_instances SET state='update_rejected' WHERE id=$1")
        .bind(command.instance_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let worker = role_pool(&pool, Role::Worker).await;
    let unset = PgRunRepository::new(worker.clone()).with_vm_owner_scope(scope());
    let vm = VmId(command.run_id.to_string());
    assert!(
        unset
            .create_run_with_vm_plan(&command, &scope(), &vm)
            .await
            .is_err()
    );
    profile_support::absent(&pool, &command).await;
    let owned = unset.with_execution_profile(FreshRunExecutionProfile::OwnedCanonical);
    assert!(owned.create_run(&command).await.is_err());
    profile_support::absent(&pool, &command).await;
    assert!(
        owned
            .create_run_with_vm_plan(&command, &scope(), &vm)
            .await
            .unwrap()
            .created
    );
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("owned_canonical".into(), false, true)
    );
    assert!(
        !owned
            .create_run_with_vm_plan(&command, &scope(), &vm)
            .await
            .unwrap()
            .created
    );
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private profile schema"]
async fn unspecified_fresh_producer_stays_historical_despite_schema_presence(pool: PgPool) {
    profile_support::profile_schema(&pool).await;
    let command = profile_support::git(&pool).await;
    let worker = role_pool(&pool, Role::Worker).await;
    let default = PgRunRepository::new(worker.clone());
    default.create_run(&command).await.unwrap();
    assert_eq!(
        profile_support::stored(&pool, &command).await,
        ("historical_unproven".into(), false, false)
    );
    let legacy = default.with_execution_profile(FreshRunExecutionProfile::LegacyScalar);
    assert!(legacy.create_run(&command).await.is_err());
    assert_eq!(
        profile_support::stored(&pool, &command).await.0,
        "historical_unproven"
    );
    worker.close().await;
}
