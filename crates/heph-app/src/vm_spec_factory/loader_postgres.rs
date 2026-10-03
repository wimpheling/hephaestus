//! Actual PUBLIC107 launch projection and spec construction; no VM executes.

#[path = "loader_fixture.rs"]
mod fixture;
#[path = "loader_seed.rs"]
mod seed;

use capability_domain::AuthorityHash;
use control_plane_postgres::{load_vm_launch_contract, run::VmLaunchContract};
use heph_run::VmSpecFactory;
use runtime_types::VolumeId;
use sqlx::{PgPool, migrate::Migrator};
use std::borrow::Cow;
use volume_domain::{
    RunVolumeSelection, RunVolumeSelections, VolumeMountScope, VolumeSelectionOrigin,
    effective_volume_slots,
};

use fixture::Fixture;

static MIGRATIONS: Migrator = sqlx::migrate!("../../migrations");

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL via DATABASE_URL"]
async fn published_legacy_loader_preserves_exact_pins_and_factory_contract(pool: PgPool) {
    let published = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 107)
                .cloned()
                .collect(),
        ),
        ..MIGRATIONS
    };
    published.run(&pool).await.expect("PUBLIC107 migrations");
    let legacy = Fixture::new(&pool, true).await;
    let stored = load(&pool, &legacy).await;
    assert_projection(&legacy, &stored);
    let scalar = legacy.factory.build(&legacy.run).await.unwrap();
    assert_base_spec(&legacy, &scalar);
    let selected = legacy_selection(&legacy, seed::contract_hash(true));
    let canonical = legacy
        .factory
        .build_with_volumes(&legacy.run, &selected)
        .await
        .unwrap();
    assert_base_spec(&legacy, &canonical);
    assert_eq!(scalar.command.program, canonical.command.program);
    assert_eq!(scalar.command.args, canonical.command.args);
    assert_substitution_denied(&legacy).await;
    drop(legacy);

    let stateless = Fixture::new(&pool, false).await;
    assert_projection(&stateless, &load(&pool, &stateless).await);
    let zero = RunVolumeSelections::new(stateless.identity(), Vec::new()).unwrap();
    let spec = stateless
        .factory
        .build_with_volumes(&stateless.run, &zero)
        .await
        .unwrap();
    assert_base_spec(&stateless, &spec);
    drop(stateless);
    println!(
        "REAL_VM_LAUNCH_CONTRACT public107=1 exact_pins=1 legacy_pointer=1 frozen_workload=1 scalar=1 canonical_zero=1 canonical_legacy=1 substitutions_denied=1 no_vm=1"
    );
}

async fn load(pool: &PgPool, fixture: &Fixture) -> VmLaunchContract {
    load_vm_launch_contract(pool, fixture.run.id.as_uuid())
        .await
        .unwrap()
        .unwrap()
}

fn assert_projection(fixture: &Fixture, stored: &VmLaunchContract) {
    assert_eq!(stored.run_id, fixture.run.id.as_uuid());
    assert_eq!(stored.instance_id, fixture.run.instance_id.as_uuid());
    assert_eq!(
        stored.instance_revision_id,
        fixture.run.instance_revision_id.as_uuid()
    );
    assert_eq!(stored.release_id, fixture.run.release_id.as_uuid());
    assert_eq!(
        stored.release_agent_id,
        fixture.run.release_agent_id.as_uuid()
    );
    assert_eq!(stored.project_id, fixture.project);
    assert_eq!(
        stored.runtime_contract_hash,
        seed::contract_hash(fixture.run.requires_state)
    );
    assert_eq!(stored.volume_mode, "legacy");
    assert_eq!(stored.state_volume_id, fixture.volume);
    assert_eq!(
        stored.runtime_contract,
        seed::runtime_contract(fixture.run.requires_state)
    );
    assert_eq!(stored.effective_runtime_policy, seed::effective_policy());
    assert_eq!(stored.requires_state, fixture.run.requires_state);
    assert_eq!(stored.release_state, "published");
    assert!(stored.revision_runnable);
    assert!(stored.attachment_runnable);
    assert!(stored.update_hook.is_none());
    assert!(stored.agent_update_id.is_none());
}

fn assert_base_spec(fixture: &Fixture, spec: &heph_runtime::VmSpec) {
    assert_eq!(spec.id.0, fixture.run.id.to_string());
    assert_eq!(spec.command.program, "/release/application.py");
    assert_eq!(spec.command.args, ["inspect"]);
    assert!(spec.disks.is_empty());
    assert!(spec.guest_volumes.is_empty());
    assert!(spec.mounts.is_empty());
    assert!(spec.runtime_authority.is_none());
    assert_eq!(spec.resources.vcpus, 1);
    assert_eq!(spec.resources.memory_mib, 512);
}

fn legacy_selection(fixture: &Fixture, hash: [u8; 32]) -> RunVolumeSelections {
    let declaration = effective_volume_slots(&[], true).unwrap().remove(0);
    let scope = VolumeMountScope::new(
        fixture.run.instance_id,
        fixture.run.instance_revision_id,
        fixture.run.release_agent_id,
        declaration.slot().clone(),
        VolumeId::from_uuid(fixture.volume.unwrap()),
        declaration.access_mode(),
        AuthorityHash::from_bytes(hash),
    )
    .unwrap();
    let selected = RunVolumeSelection::new(
        fixture.identity(),
        scope,
        declaration,
        VolumeSelectionOrigin::LegacyOrigin,
    )
    .unwrap();
    RunVolumeSelections::new(fixture.identity(), vec![selected]).unwrap()
}

async fn assert_substitution_denied(fixture: &Fixture) {
    let exact = legacy_selection(fixture, seed::contract_hash(true));
    let mut wrong_run = fixture.run.clone();
    wrong_run.instance_revision_id = runtime_types::AgentInstanceRevisionId::new();
    assert!(
        fixture
            .factory
            .build_with_volumes(&wrong_run, &exact)
            .await
            .is_err()
    );
    let identity = fixture.identity();
    let wrong_identity = volume_domain::RunVolumeIdentity::new(
        identity.run_id(),
        identity.instance_id(),
        runtime_types::AgentInstanceRevisionId::new(),
        identity.release_id(),
        identity.release_agent_id(),
        identity.project_id(),
    )
    .unwrap();
    let wrong_identity = RunVolumeSelections::new(wrong_identity, Vec::new()).unwrap();
    assert!(
        fixture
            .factory
            .build_with_volumes(&fixture.run, &wrong_identity)
            .await
            .is_err()
    );
    let wrong_hash = legacy_selection(fixture, [99; 32]);
    assert!(
        fixture
            .factory
            .build_with_volumes(&fixture.run, &wrong_hash)
            .await
            .is_err()
    );
}
