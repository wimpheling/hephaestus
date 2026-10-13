use crate::fixture::{Fixture, finish_held, snapshot};
use recipe_application::{DiagnosticCode, ResourceAction};
use recipe_postgres::VerifiedOutcome;
use sqlx::{PgPool, postgres::PgDatabaseError};
use uuid::Uuid;
use volume_trait::VolumeOwnedProvisioningRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn missing111_and_ready_before_completion_deny_raw_scalar_and_named_leases_including_uncertain(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    let (claim, request) = f.request().await;
    let volume = request
        .purpose
        .receipt()
        .intent
        .registration()
        .id()
        .as_uuid();
    let before = snapshot(&pool).await;
    assert_raw_denial(&f, volume, "historical_unproven").await;
    assert_raw_denial(&f, volume, "legacy_declaration").await;
    assert_eq!(
        snapshot(&pool).await,
        before,
        "missing111 is not an unrelated unrestricted birth"
    );
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    let before = snapshot(&pool).await;
    assert_raw_denial(&f, volume, "historical_unproven").await;
    assert_raw_denial(&f, volume, "legacy_declaration").await;
    assert_eq!(snapshot(&pool).await, before);
    let denied = f
        .actor
        .assert_owned_first_ready_verification(&ready.claim)
        .await;
    assert!(
        matches!(denied, Err(volume_trait::VolumeError::PermissionDenied)),
        "worker role checked before business lookup"
    );
    let privilege:(bool,bool)=sqlx::query_as("SELECT has_function_privilege('hephaestus_worker','assert_recipe_owned_first_ready(uuid,uuid,bigint,bytea)','EXECUTE'),has_function_privilege('hephaestus_app','assert_recipe_owned_first_ready(uuid,uuid,bigint,bytea)','EXECUTE')").fetch_one(&pool).await.unwrap();
    assert_eq!(privilege, (true, false));
    f.h.repository
        .mark_resource_ambiguous(&f.h.actor, &claim, DiagnosticCode::ProviderOutcomeUnknown)
        .await
        .unwrap();
    assert_raw_denial(&f, volume, "historical_unproven").await;
    assert_raw_denial(&f, volume, "legacy_declaration").await;
    assert_eq!(f.count("formats"), 1);
    assert_eq!(f.count("fscks"), 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn genuine_completion_removes_only_first_birth_barrier_existing_named_policy_still_denies(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    let (claim, request) = f.request().await;
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    let volume = request
        .purpose
        .receipt()
        .intent
        .registration()
        .id()
        .as_uuid();
    let custody = f.local.verify_owned_ready(&ready.claim).await.unwrap();
    finish_held(&f, claim, custody).await;
    let (mapped, complete): (bool, bool) = sqlx::query_as(
        "SELECT recipe_owned_first_birth_matches($1),recipe_owned_first_birth_completed($1)",
    )
    .bind(volume)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        mapped && complete,
        "exact original immutable Applied/Create+Ready is recognized"
    );
    let before = snapshot(&pool).await;
    let error = raw_lease(&f, volume, "legacy_declaration")
        .await
        .unwrap_err();
    let database = error
        .as_database_error()
        .unwrap()
        .downcast_ref::<PgDatabaseError>();
    assert_eq!(database.code(), "23000");
    assert_eq!(
        database.message(),
        "cannot manufacture historical run lease evidence",
        "successful birth removes its barrier but cannot manufacture admitted Run provenance"
    );
    let named_supported: bool = sqlx::query_scalar("SELECT named_volume_dispatch_supported($1)")
        .bind(f.h.fixture.instance)
        .fetch_one(&f.h.worker)
        .await
        .unwrap();
    assert!(
        !named_supported,
        "ordinary named policy remains explicitly unsupported"
    );
    assert_eq!(
        snapshot(&pool).await,
        before,
        "completion grants no named dispatch/lease capability"
    );
    let unrelated:bool=sqlx::query_scalar("SELECT recipe_owned_first_birth_completed($1) OR EXISTS(SELECT 1 FROM recipe_owned_volume_preparations WHERE volume_id=$1)").bind(f.h.fixture.volume).fetch_one(&pool).await.unwrap();
    assert!(
        !unrelated,
        "existing uncorrelated volume remains outside the new guard class"
    );
    assert!(f.local.verify_owned_ready(&ready.claim).await.is_err());
    assert_eq!(f.count("fscks"), 2, "completed stale call cannot fsck");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn inactive_failed_or_remove_closed_original_create_cannot_reverify_ready(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let (claim, request) = f.request().await;
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    let recorder = recipe_postgres::PostgresEffectVerificationRecorder::new(f.h.worker.clone());
    recorder
        .record(
            &claim,
            claim.provenance.attempt_id,
            VerifiedOutcome::Failed(DiagnosticCode::ProviderFailure),
        )
        .await
        .unwrap();
    f.h.repository
        .complete_resource_effect(
            &f.h.actor,
            &claim,
            recipe_application::EffectEvidence {
                identity: claim.identity,
                input_hash: claim.input_hash,
                outcome: recipe_application::EffectOutcome::Failed(DiagnosticCode::ProviderFailure),
            },
        )
        .await
        .unwrap();
    let before = snapshot(&pool).await;
    assert!(
        f.local.verify_owned_ready(&ready.claim).await.is_err(),
        "Failed is not active first completion permission"
    );
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(f.count("fscks"), 1);
    let inspected =
        f.h.repository
            .inspect(&f.h.actor, f.h.intent.id())
            .await
            .unwrap();
    f.h.repository
        .admit_remove(
            &f.h.actor,
            crate::fixtures::remove(&f.h.actor, f.h.intent.id(), inspected.version),
        )
        .await
        .unwrap();
    let before = snapshot(&pool).await;
    assert!(
        f.local.verify_owned_ready(&ready.claim).await.is_err(),
        "Remove permanently closes Install; Ready removal is a separate qualified path"
    );
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(f.count("fscks"), 1);
    assert_eq!(claim.action, ResourceAction::Create);
    assert_raw_denial(
        &f,
        request
            .purpose
            .receipt()
            .intent
            .registration()
            .id()
            .as_uuid(),
        "legacy_declaration",
    )
    .await;
}
async fn assert_raw_denial(f: &Fixture, volume: Uuid, origin: &str) {
    let error = raw_lease(f, volume, origin).await.unwrap_err();
    let database = error
        .as_database_error()
        .unwrap()
        .downcast_ref::<PgDatabaseError>();
    assert_eq!(database.code(), "23000");
    assert_eq!(
        database.message(),
        "original protected owned Create must complete before runtime lease admission"
    );
}
async fn raw_lease(
    f: &Fixture,
    volume: Uuid,
    origin: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,update_id,host_id,fencing_token,state,expires_at,selection_origin) VALUES($1,$2,$3,$4,'creator-fixture-host',1,'active',now()+interval '30 seconds',$5)")
        .bind(Uuid::new_v4()).bind(volume).bind(f.h.fixture.instance).bind(Uuid::new_v4()).bind(origin).execute(&f.h.worker).await
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn native_pgcrypto_exact_uuidv5_and_readonly_fence_reject_wrong_original_operation(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    let (_, mut request) = f.request().await;
    let intent = request.purpose.receipt().intent.clone();
    for value in [
        "00000000-0000-4000-8000-000000000001",
        "ffffffff-ffff-4fff-bfff-ffffffffffff",
        "12345678-1234-5678-9234-567812345678",
    ] {
        let id = Uuid::parse_str(value).unwrap();
        let mut wire = intent.creation().fields().clone();
        wire.operation_id = volume_trait::VolumeCreationOperationId::from_uuid(id).unwrap();
        let expected = volume_trait::OwnedProvisioningExpectation::new(
            volume_trait::OwnedVolumeRegistration::new(
                intent.registration().clone(),
                volume_trait::VolumeCreationIdentity::try_from(wire).unwrap(),
            ),
            f.owner.host().into(),
            f.root.path().into(),
            f.owner.namespace_id(),
        )
        .unwrap();
        let actual: Uuid = sqlx::query_scalar("SELECT recipe_owned_first_provision_id($1)")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(actual, expected.first_operation_id());
        assert_ne!(
            actual,
            Uuid::new_v5(&id, b"heph-owned-first-provision-v1"),
            "the terminating NUL is part of the immutable mapping"
        );
    }
    let original = request.operation_id;
    request.operation_id = Uuid::new_v4();
    assert_ne!(request.operation_id, original);
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    let before = snapshot(&pool).await;
    assert!(
        f.worker
            .assert_owned_first_ready_verification(&ready.claim)
            .await
            .is_err(),
        "a different gen1 operation is not the mapped first operation"
    );
    assert!(f.local.verify_owned_ready(&ready.claim).await.is_err());
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 1));
}
