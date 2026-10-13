use super::fixture::{assert_original, begin, identity, intent, setup, snapshot};
use crate::provisioning::observation;
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedBackingPhase, OwnedProvisioningExpectation, OwnedVolumeRegistration,
    VolumeCreationIdentity, VolumeCreationOperationId, VolumeError,
    VolumeOwnedProvisioningRepository, VolumeResourceRepository, VolumeRootNamespaceId,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn complete_original_creation_and_geometry_pins_are_compared(pool: PgPool) {
    let (fixture, expected, request) = setup(&pool).await;
    begin(&fixture, &request).await;
    let before = snapshot(&pool, &expected).await;
    for field in 0..11 {
        let registration = expected.intent();
        let mut wire = registration.creation().fields().clone();
        match field {
            0 => wire.operation_id = VolumeCreationOperationId::from_uuid(Uuid::new_v4()).unwrap(),
            1 => wire.command_id = Uuid::new_v4(),
            2 => wire.effect_id = Uuid::new_v4(),
            3 => wire.attempt_id = Uuid::new_v4(),
            4 => wire.actor_id = fixture.successor.as_uuid(),
            5 => wire.request_id = Uuid::new_v4(),
            6 => wire.resource_version += 1,
            7 => wire.generation += 1,
            8 => wire.input_hash = [3; 32],
            9 => wire.intent_hash = [4; 32],
            _ => {
                wire.scope_id =
                    volume_trait::VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).unwrap();
            }
        }
        let changed = OwnedVolumeRegistration::new(
            registration.registration().clone(),
            VolumeCreationIdentity::try_from(wire).unwrap(),
        );
        let forged = expectation(&expected, changed);
        assert!(
            fixture
                .worker
                .discover_owned_provisioning(&forged)
                .await
                .is_err()
        );
    }
    for registration in [
        volume_trait::VolumeRegistration::new(
            expected.intent().registration().id(),
            fixture.project,
            33_554_432,
            expected.intent().registration().filesystem_uuid(),
        )
        .unwrap(),
        volume_trait::VolumeRegistration::new(
            expected.intent().registration().id(),
            fixture.project,
            16_777_216,
            Uuid::new_v4(),
        )
        .unwrap(),
        volume_trait::VolumeRegistration::new(
            expected.intent().registration().id(),
            Uuid::new_v4(),
            16_777_216,
            expected.intent().registration().filesystem_uuid(),
        )
        .unwrap(),
    ] {
        let changed =
            OwnedVolumeRegistration::new(registration, expected.intent().creation().clone());
        assert!(
            fixture
                .worker
                .discover_owned_provisioning(&expectation(&expected, changed))
                .await
                .is_err()
        );
    }
    assert_eq!(snapshot(&pool, &expected).await, before);
}

fn expectation(
    expected: &OwnedProvisioningExpectation,
    intent: OwnedVolumeRegistration,
) -> OwnedProvisioningExpectation {
    OwnedProvisioningExpectation::new(
        intent,
        expected.host().into(),
        expected.root().into(),
        expected.owner_namespace(),
    )
    .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn foreign_host_root_and_namespace_never_filter_history_into_absence(pool: PgPool) {
    let (fixture, expected, request) = setup(&pool).await;
    begin(&fixture, &request).await;
    let before = snapshot(&pool, &expected).await;
    for (host, root, namespace) in [
        (
            "foreign-host",
            expected.root().to_path_buf(),
            expected.owner_namespace(),
        ),
        (
            expected.host(),
            "/foreign-root".into(),
            expected.owner_namespace(),
        ),
        (
            expected.host(),
            expected.root().to_path_buf(),
            VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap(),
        ),
    ] {
        let foreign = OwnedProvisioningExpectation::new(
            expected.intent().clone(),
            host.into(),
            root,
            namespace,
        )
        .unwrap();
        assert!(
            fixture
                .worker
                .discover_owned_provisioning(&foreign)
                .await
                .is_err()
        );
    }
    assert_eq!(snapshot(&pool, &expected).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn missing_and_independent_resources_are_not_unadmitted_sealed_births(pool: PgPool) {
    let (fixture, expected, _) = setup(&pool).await;
    let independent = intent(&fixture);
    let absent = expectation(&expected, independent.clone());
    assert!(
        fixture
            .worker
            .discover_owned_provisioning(&absent)
            .await
            .is_err()
    );
    fixture
        .actor
        .register(&identity(fixture.owner), independent.registration())
        .await
        .unwrap();
    assert!(
        fixture
            .worker
            .discover_owned_provisioning(&absent)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn another_operation_identity_does_not_certify_original_first_admission(pool: PgPool) {
    let (fixture, expected, mut request) = setup(&pool).await;
    request.operation_id = Uuid::new_v4();
    begin(&fixture, &request).await;
    let before = snapshot(&pool, &expected).await;
    assert!(
        fixture
            .worker
            .discover_owned_provisioning(&expected)
            .await
            .is_err()
    );
    assert_eq!(snapshot(&pool, &expected).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn legitimate_multiple_operation_history_remains_held_without_new_cas(pool: PgPool) {
    let (fixture, expected, mut request) = setup(&pool).await;
    let first = begin(&fixture, &request).await;
    fixture
        .worker
        .record_owned_observation(
            &first.claim,
            &observation(&first.claim, OwnedBackingPhase::FailedBeforeFormat),
        )
        .await
        .unwrap();
    request.operation_id = Uuid::new_v4();
    request.expected_generation = 1;
    let second = begin(&fixture, &request).await;
    assert_eq!(second.claim.generation, 2);
    let before = snapshot(&pool, &expected).await;
    assert!(matches!(
        fixture.worker.discover_owned_provisioning(&expected).await,
        Err(VolumeError::ProvisioningUncertain(_))
    ));
    assert_eq!(snapshot(&pool, &expected).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private checkpoint111"]
async fn actor_discovery_denies_before_lookup_and_orphan_transition_cannot_commit(pool: PgPool) {
    let (fixture, expected, request) = setup(&pool).await;
    let original = begin(&fixture, &request).await;
    let before = snapshot(&pool, &expected).await;
    assert!(matches!(
        fixture.actor.discover_owned_provisioning(&expected).await,
        Err(VolumeError::PermissionDenied)
    ));
    let worker = crate::seals::role_pool(&pool, true).await;
    let mut tx = worker.begin().await.unwrap();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO volume_owned_transition_proofs(id,volume_id,operation_id,generation,previous_generation,mode) VALUES($1,$2,$3,1,1,'recorded')")
        .bind(id).bind(expected.intent().registration().id().as_uuid()).bind(original.claim.operation_id)
        .execute(&mut *tx).await.unwrap();
    assert!(
        tx.commit().await.is_err(),
        "no committed worker observation for orphan proof"
    );
    assert_original(&fixture, &expected, &original).await;
    assert_eq!(snapshot(&pool, &expected).await, before);
}
