//! Orchestration only: expected upgrade values come from sealed BEFORE evidence.
use super::{
    catalog, catalog_constraints, catalog_defaults, catalog_routines, catalog_stability,
    catalog_structure, catalog_table_columns, events, fixture, history, journal,
    new_fields::{self, Qualification, Selection},
    preservation,
    snapshots::{self, Phase, Snapshot},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
#[path = "harness/replay143.rs"]
mod replay;
use replay::{prefix137, replay143};

pub(super) async fn fresh(pool: &PgPool) -> Result<(), sqlx::Error> {
    fixture::run143(pool).await;
    let installed = snapshots::capture(pool, Phase::Public143, None).await?;
    let installed_journal = journal::validate_fresh(&installed)?;
    assert_eq!(installed_journal.phase(), Phase::Public143);
    let installed_catalog = catalog::capture(pool).await?;
    super::public139::assert_worker_contract(pool).await?;
    fixture::run143(pool).await;
    let replay = snapshots::capture(pool, Phase::Public143, None).await?;
    let replay_journal = journal::validate_fresh(&replay)?;
    installed_journal.assert_replay(&replay_journal)?;
    // Fresh captures have no fabricated original107 projections. Full equality
    // covers all184 tables' columns/JSONB/text rows and all10 view shapes.
    assert!(installed == replay, "fresh143 complete row replay changed");
    let replay_catalog = catalog::capture(pool).await?;
    catalog_stability::assert_replay(&installed_catalog, &replay_catalog);
    Ok(())
}

pub(super) async fn populated(pool: &PgPool, reference_pool: &PgPool) -> Result<(), sqlx::Error> {
    let reference_catalog = fresh_reference(reference_pool).await?;

    let seeded = fixture::populated107(pool).await;
    let before = snapshots::capture(pool, Phase::Public107, None).await?;
    let before_journal = journal::validate(&before, None)?;
    assert_eq!(before_journal.phase(), Phase::Public107);
    let qualification = new_fields::qualify(&before)?;
    assert_history(&seeded, &qualification);
    let before_catalog = catalog::capture(pool).await?;

    // This genuine prefix checkpoint exercises run137 meaningfully. The final
    // full143 invocation still uses the ordinary unmodified complete Migrator.
    prefix137(
        pool,
        &qualification,
        &before,
        &before_journal,
        &before_catalog,
    )
    .await?;
    fixture::run138(pool).await;
    let checkpoint = snapshots::capture(pool, Phase::Public138, Some(&before)).await?;
    let checkpoint_journal = journal::validate(&checkpoint, Some(&before_journal))?;
    let checkpoint_catalog = catalog::capture(pool).await?;
    assert_upgrade(
        pool,
        &qualification,
        &checkpoint,
        &before_journal,
        &before_catalog,
        &checkpoint_catalog,
    )
    .await?;
    let (public140, public140_catalog) = forward_checkpoints(
        pool,
        &qualification,
        &before,
        &before_journal,
        &before_catalog,
        &checkpoint_journal,
    )
    .await?;
    let public140_journal = journal::validate(&public140, Some(&before_journal))?;
    fixture::run141(pool).await;
    let public141 = snapshots::capture(pool, Phase::Public141, Some(&before)).await?;
    let public141_journal = journal::validate(&public141, Some(&before_journal))?;
    public140_journal.assert_forward_prefix(&public141_journal)?;
    let public141_catalog = catalog::capture(pool).await?;
    super::forward::assert141(&public140_catalog, &public141_catalog);
    fixture::run142(pool).await;
    let public142 = snapshots::capture(pool, Phase::Public142, Some(&before)).await?;
    let public142_journal = journal::validate(&public142, Some(&before_journal))?;
    public141_journal.assert_forward_prefix(&public142_journal)?;
    let public142_catalog = catalog::capture(pool).await?;
    super::forward::assert142(&public141_catalog, &public142_catalog);
    fixture::run143(pool).await;
    let upgraded = snapshots::capture(pool, Phase::Public143, Some(&before)).await?;
    let upgraded_catalog = catalog::capture(pool).await?;

    assert_upgrade(
        pool,
        &qualification,
        &upgraded,
        &before_journal,
        &before_catalog,
        &upgraded_catalog,
    )
    .await?;

    let upgraded_journal = journal::validate(&upgraded, Some(&before_journal))?;
    assert_eq!(upgraded_journal.phase(), Phase::Public143);
    public142_journal.assert_forward_prefix(&upgraded_journal)?;
    super::public139::assert_worker_contract(pool).await?;
    catalog_routines::assert_routines(&before_catalog, &upgraded_catalog, &reference_catalog);
    catalog_table_columns::assert_table_columns(
        &before_catalog,
        &upgraded_catalog,
        &reference_catalog,
    );
    catalog_defaults::assert_defaults(&before_catalog, &upgraded_catalog, &reference_catalog);
    catalog_constraints::assert_constraints(&before_catalog, &upgraded_catalog, &reference_catalog);
    catalog_structure::assert_structure(&before_catalog, &upgraded_catalog, &reference_catalog);
    super::forward::assert143(&public142_catalog, &upgraded_catalog);

    replay143(
        pool,
        &before,
        &before_journal,
        &upgraded,
        &upgraded_journal,
        &upgraded_catalog,
    )
    .await?;
    Ok(())
}

async fn forward_checkpoints(
    pool: &PgPool,
    qualification: &Qualification<'_>,
    before: &Snapshot,
    before_journal: &journal::Journal<'_>,
    before_catalog: &catalog::Snapshot,
    checkpoint_journal: &journal::Journal<'_>,
) -> Result<(Snapshot, catalog::Snapshot), sqlx::Error> {
    fixture::run139(pool).await;
    let public139 = snapshots::capture(pool, Phase::Public139, Some(before)).await?;
    let public139_journal = journal::validate(&public139, Some(before_journal))?;
    checkpoint_journal.assert_public138_prefix(&public139_journal)?;
    let public139_catalog = catalog::capture(pool).await?;
    assert_upgrade(
        pool,
        qualification,
        &public139,
        before_journal,
        before_catalog,
        &public139_catalog,
    )
    .await?;
    fixture::run140(pool).await;
    let public140 = snapshots::capture(pool, Phase::Public140, Some(before)).await?;
    let public140_journal = journal::validate(&public140, Some(before_journal))?;
    public139_journal.assert_forward_prefix(&public140_journal)?;
    let public140_catalog = catalog::capture(pool).await?;
    assert_upgrade(
        pool,
        qualification,
        &public140,
        before_journal,
        before_catalog,
        &public140_catalog,
    )
    .await?;
    super::forward::assert140(&public139_catalog, &public140_catalog);
    Ok((public140, public140_catalog))
}

async fn fresh_reference(reference_pool: &PgPool) -> Result<catalog::Snapshot, sqlx::Error> {
    fixture::run143(reference_pool).await;
    let reference_rows = snapshots::capture(reference_pool, Phase::Public143, None).await?;
    let reference_journal = journal::validate_fresh(&reference_rows)?;
    assert_eq!(reference_journal.phase(), Phase::Public143);
    catalog::capture(reference_pool).await
}

async fn assert_upgrade(
    pool: &PgPool,
    qualification: &Qualification<'_>,
    after: &Snapshot,
    before_journal: &journal::Journal<'_>,
    before_catalog: &catalog::Snapshot,
    after_catalog: &catalog::Snapshot,
) -> Result<(), sqlx::Error> {
    let admitted = journal::validate(after, Some(before_journal))?;
    assert_eq!(admitted.phase(), after.phase());
    preservation::assert_upgrade(qualification.before(), after);
    new_fields::assert_values(qualification.before(), after)?;
    events::assert_upgrade(pool, qualification, after).await?;
    catalog_stability::assert_untouched(before_catalog, after_catalog);
    Ok(())
}

fn assert_history(seeded: &history::History, qualification: &Qualification<'_>) {
    let before = qualification.before();
    assert_instance(before, &seeded.qualifying_instance);
    assert_instance(before, &seeded.unknown_instance);
    assert_ne!(
        seeded.qualifying_instance.instance,
        seeded.unknown_instance.instance
    );
    assert_lease(before, &seeded.qualifying_lease);
    assert_lease(before, &seeded.unknown_lease);
    assert_eq!(
        seeded.qualifying_lease.run,
        seeded.qualifying_instance.normal
    );
    assert_eq!(seeded.unknown_lease.run, seeded.update);
    assert_eq!(
        row(before, "runs", seeded.update.as_uuid())["run_kind"],
        "update"
    );
    assert_eq!(seeded.qualifying_before.len(), 1);
    assert_eq!(qualification.selections().len(), 1);
    assert!(
        !qualification
            .selections()
            .contains_key(&seeded.unknown_lease.id)
    );
    let selected = &seeded.qualifying_before[0];
    assert_selection(
        selected,
        &qualification.selections()[&selected.lease],
        before,
    );
    assert_eq!(selected.lease, seeded.qualifying_lease.id);
    assert_eq!(selected.volume, seeded.qualifying_lease.volume);
    assert_eq!(
        selected.instance,
        seeded.qualifying_instance.instance.as_uuid()
    );
    assert_seed_rows(seeded, before);
}

fn assert_instance(before: &Snapshot, instance: &history::InstanceHistory) {
    let run = row(before, "runs", instance.normal.as_uuid());
    for (field, expected) in [
        ("instance_id", instance.instance.as_uuid()),
        ("instance_revision_id", instance.revision.as_uuid()),
        ("attachment_id", instance.attachment),
        ("release_id", instance.release),
        ("release_agent_id", instance.agent),
    ] {
        assert_eq!(run[field], json!(expected));
    }
    assert_eq!(run["run_kind"], "normal");
    assert_eq!(
        row(before, "agent_attachments", instance.attachment)["instance_id"],
        json!(instance.instance.as_uuid())
    );
    assert_eq!(
        row(
            before,
            "agent_instance_revisions",
            instance.revision.as_uuid()
        )["instance_id"],
        json!(instance.instance.as_uuid())
    );
}

fn assert_lease(before: &Snapshot, lease: &history::LeaseHistory) {
    let observed = row(before, "agent_instance_volume_leases", lease.id);
    assert_eq!(observed["volume_id"], json!(lease.volume));
    assert_eq!(observed["instance_id"], json!(lease.instance.as_uuid()));
    assert_eq!(observed["run_id"], json!(lease.run.as_uuid()));
    assert_eq!(observed["host_id"], lease.host);
    assert_eq!(observed["fencing_token"], lease.fence);
    assert_eq!(lease.fence, 7);
    assert_eq!(observed["state"], "active");
    assert_eq!(
        row(before, "agent_instances", lease.instance.as_uuid())["state_volume_id"],
        json!(lease.volume)
    );
}

fn assert_selection(selected: &history::LegacySelection, sealed: &Selection, before: &Snapshot) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    assert_eq!(sealed.instance(), selected.instance);
    assert_eq!(sealed.project(), selected.project);
    for (field, expected) in [
        ("selected_revision_id", selected.revision),
        ("selected_release_id", selected.release),
        ("selected_release_agent_id", selected.agent),
        ("selected_project_id", selected.project),
    ] {
        assert_eq!(sealed.fields()[field], json!(expected));
    }
    assert_eq!(
        row(before, "agent_instance_volume_leases", selected.lease)["volume_id"],
        json!(selected.volume)
    );
    assert_eq!(selected.contract_hash.len(), 32);
    let mut hash = String::from(r"\x");
    for byte in &selected.contract_hash {
        hash.push(char::from(HEX[usize::from(*byte >> 4)]));
        hash.push(char::from(HEX[usize::from(*byte & 15)]));
    }
    assert_eq!(sealed.fields()["release_contract_hash"], hash);
}

fn assert_seed_rows(seeded: &history::History, before: &Snapshot) {
    assert_eq!(seeded.before.len(), history::TABLES.len());
    for table in history::TABLES {
        let expected = &seeded.before[table];
        if table == "melange_tuples" {
            // Its seeded rows are retained by History; Snapshot captures view
            // shape rather than claiming to snapshot view-result row values.
            assert!(before.views().contains_key(table));
            continue;
        }
        let mut actual: Vec<_> = before.tables()[table]
            .rows
            .iter()
            .map(|row| row.json.clone())
            .collect();
        actual.sort_by_key(Value::to_string);
        assert!(actual == *expected, "seeded BEFORE table differs: {table}");
    }
}

fn row<'a>(snapshot: &'a Snapshot, table: &str, id: Uuid) -> &'a Value {
    let expected = json!(id);
    let mut matches = snapshot.tables()[table]
        .rows
        .iter()
        .filter(|row| row.json["id"] == expected);
    let found = matches.next().expect("actual seeded identity missing");
    assert!(matches.next().is_none(), "duplicate actual seeded identity");
    &found.json
}
