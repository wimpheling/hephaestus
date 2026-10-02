#[path = "volumes_fixture.rs"]
mod fixture;

use capability_domain::CapabilitySlotKey;
use uuid::Uuid;
use volume_domain::{
    GuestMountPath, VolumeAccessMode, VolumeSelectionOrigin, VolumeSlotDeclaration,
};

use super::validate;
use fixture::{catalog, declaration, fixture, selection, set};

#[test]
fn empty_catalog_and_optional_absence_use_exact_persisted_pins() {
    let (run, mut stored) = fixture();
    assert!(validate(&run, &stored, &set(&stored, vec![])).is_ok());
    catalog(&mut stored, &[declaration(false)]);
    assert!(validate(&run, &stored, &set(&stored, vec![])).is_ok());
    catalog(&mut stored, &[declaration(true)]);
    assert!(validate(&run, &stored, &set(&stored, vec![])).is_err());
}

#[test]
fn named_selection_validates_the_full_frozen_declaration() {
    let (run, mut stored) = fixture();
    let declared = declaration(true);
    catalog(&mut stored, std::slice::from_ref(&declared));
    let selected = selection(&stored, declared, VolumeSelectionOrigin::Explicit);
    let selections = set(&stored, vec![selected]);
    assert!(validate(&run, &stored, &selections).is_ok());
    for substitute in [
        changed(
            "data",
            "/data",
            VolumeAccessMode::ReadOnly,
            true,
            16_777_216,
        ),
        changed(
            "data",
            "/other",
            VolumeAccessMode::ReadWrite,
            true,
            16_777_216,
        ),
        changed(
            "data",
            "/data",
            VolumeAccessMode::ReadWrite,
            false,
            16_777_216,
        ),
        changed(
            "data",
            "/data",
            VolumeAccessMode::ReadWrite,
            true,
            32_000_000,
        ),
        changed(
            "extra",
            "/data",
            VolumeAccessMode::ReadWrite,
            true,
            16_777_216,
        ),
    ] {
        catalog(&mut stored, &[substitute]);
        assert!(validate(&run, &stored, &selections).is_err());
    }
}

#[test]
fn selected_project_source_revision_and_hash_cannot_be_substituted() {
    let (mut run, mut stored) = fixture();
    let declared = declaration(true);
    catalog(&mut stored, std::slice::from_ref(&declared));
    let selections = set(
        &stored,
        vec![selection(
            &stored,
            declared,
            VolumeSelectionOrigin::Explicit,
        )],
    );
    let original_source = run.release_id;
    run.release_id = runtime_types::ReleaseId::new();
    assert!(validate(&run, &stored, &selections).is_err());
    run.release_id = original_source;
    let original_revision = stored.instance_revision_id;
    stored.instance_revision_id = Uuid::new_v4();
    assert!(validate(&run, &stored, &selections).is_err());
    stored.instance_revision_id = original_revision;
    let original_project = stored.project_id;
    stored.project_id = Uuid::new_v4();
    assert!(validate(&run, &stored, &selections).is_err());
    stored.project_id = original_project;
    stored.runtime_contract_hash = vec![8; 32];
    assert!(validate(&run, &stored, &selections).is_err());
    stored.runtime_contract_hash.clear();
    assert!(validate(&run, &stored, &selections).is_err());
}

#[test]
fn frozen_legacy_declaration_is_distinct_from_an_authored_state_slot() {
    let (run, mut stored) = fixture();
    stored.requires_state = true;
    stored.runtime_contract["requires_state"] = true.into();
    let state = volume_domain::effective_volume_slots(&[], true)
        .unwrap()
        .remove(0);
    let implicit = set(
        &stored,
        vec![selection(
            &stored,
            state.clone(),
            VolumeSelectionOrigin::LegacyDeclaration,
        )],
    );
    assert!(validate(&run, &stored, &implicit).is_ok());
    let explicit = set(
        &stored,
        vec![selection(
            &stored,
            state.clone(),
            VolumeSelectionOrigin::Explicit,
        )],
    );
    assert!(validate(&run, &stored, &explicit).is_err());
    stored.requires_state = false;
    stored.runtime_contract["requires_state"] = false.into();
    catalog(&mut stored, &[state]);
    assert!(validate(&run, &stored, &explicit).is_ok());
    assert!(validate(&run, &stored, &implicit).is_err());
}

#[test]
fn legacy_origin_requires_actual_mode_and_exact_scalar_pointer() {
    let (run, mut stored) = fixture();
    stored.requires_state = true;
    stored.runtime_contract["requires_state"] = true.into();
    stored.volume_mode = "legacy".into();
    stored.state_volume_id = Some(Uuid::new_v4());
    let state = volume_domain::effective_volume_slots(&[], true)
        .unwrap()
        .remove(0);
    let selections = set(
        &stored,
        vec![selection(
            &stored,
            state,
            VolumeSelectionOrigin::LegacyOrigin,
        )],
    );
    assert!(validate(&run, &stored, &selections).is_ok());
    stored.state_volume_id = Some(Uuid::new_v4());
    assert!(validate(&run, &stored, &selections).is_err());
    stored.state_volume_id = None;
    assert!(validate(&run, &stored, &selections).is_err());
    stored.volume_mode = "named".into();
    assert!(validate(&run, &stored, &selections).is_err());
}

#[test]
fn malformed_catalog_and_contradictory_frozen_requirement_fail_closed() {
    let (run, mut stored) = fixture();
    let empty = set(&stored, vec![]);
    stored.runtime_contract["volume_slots"] = serde_json::json!([{
        "slot":"data","guest_path":"/data","access_mode":"read_write",
        "required":false,"minimum_capacity_bytes":1,"unknown":true
    }]);
    assert!(validate(&run, &stored, &empty).is_err());
    catalog(&mut stored, &[declaration(false), declaration(false)]);
    assert!(validate(&run, &stored, &empty).is_err());
    stored.runtime_contract["volume_slots"] = serde_json::json!({});
    assert!(validate(&run, &stored, &empty).is_err());
    catalog(&mut stored, &[]);
    stored.runtime_contract["requires_state"] = true.into();
    assert!(validate(&run, &stored, &empty).is_err());
}

fn changed(
    slot: &str,
    path: &str,
    mode: VolumeAccessMode,
    required: bool,
    capacity: u64,
) -> VolumeSlotDeclaration {
    VolumeSlotDeclaration::new(
        CapabilitySlotKey::parse(slot).unwrap(),
        GuestMountPath::parse(path).unwrap(),
        mode,
        required,
        capacity,
    )
    .unwrap()
}

#[tokio::test]
async fn common_workload_builder_preserves_scalar_output_without_adding_mounts() {
    use std::collections::BTreeMap;

    use super::super::{PgAgentVmSpecFactory, RootFilesystem, RuntimePolicy};
    let (run, stored) = fixture();
    let factory = PgAgentVmSpecFactory {
        pool: sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://fixture:fixture@localhost/unused")
            .unwrap(),
        root_images: BTreeMap::from([(
            "test-image".into(),
            RootFilesystem::Directory {
                host_path: "/unused-image".into(),
            },
        )]),
        runtime_policy: RuntimePolicy {
            version: "test/v1".into(),
            max_vcpus: 2,
            max_memory_mib: 1_024,
            allow_broker_only: false,
            allow_egress: false,
        },
    };
    let empty = set(&stored, vec![]);
    validate(&run, &stored, &empty).unwrap();
    let spec = factory.build_stored(&run, stored).unwrap();
    assert_eq!(spec.command.program, "/release/app.py");
    assert_eq!(spec.command.args, ["inspect"]);
    assert_eq!(spec.id.0, run.id.to_string());
    assert!(spec.guest_volumes.is_empty());
    assert!(spec.disks.is_empty());
    assert!(spec.mounts.is_empty());
    assert!(spec.runtime_authority.is_none());
    let (scalar_run, mut scalar_stored) = fixture();
    // The scalar builder retains its previous catalog-independent behavior;
    // only the complete-set entry point accepts catalog evidence.
    scalar_stored.runtime_contract["volume_slots"] = serde_json::json!({"malformed":true});
    assert!(factory.build_stored(&scalar_run, scalar_stored).is_ok());
}

#[test]
fn a_second_required_slot_cannot_be_ignored_or_replaced_by_a_foreign_slot() {
    let (run, mut stored) = fixture();
    let data = declaration(true);
    let second = changed("logs", "/logs", VolumeAccessMode::ReadOnly, true, 1);
    catalog(&mut stored, &[data.clone(), second.clone()]);
    let data_selection = selection(&stored, data, VolumeSelectionOrigin::Explicit);
    let missing = set(&stored, vec![data_selection.clone()]);
    assert!(validate(&run, &stored, &missing).is_err());
    let complete = set(
        &stored,
        vec![
            data_selection.clone(),
            selection(&stored, second, VolumeSelectionOrigin::Explicit),
        ],
    );
    assert!(validate(&run, &stored, &complete).is_ok());
    let foreign = changed("foreign", "/foreign", VolumeAccessMode::ReadOnly, true, 1);
    let extra = set(
        &stored,
        vec![
            data_selection,
            selection(&stored, foreign, VolumeSelectionOrigin::Explicit),
        ],
    );
    assert!(validate(&run, &stored, &extra).is_err());
}
