use hephaestus_app::recipe_instance_import::to_import_command;
use sqlx::PgPool;

use crate::instance_fixture::Harness;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn volume_converter_preserves_every_frozen_typed_field_without_provider_effects(
    pool: PgPool,
) {
    let h = Harness::new(&pool, true).await;
    let prepared = h.prepared().await;
    let command = to_import_command(&prepared);
    assert_eq!(command.command_key, prepared.command_key());
    assert_eq!(
        command.instance_id.as_uuid(),
        prepared.instance_id().as_uuid()
    );
    assert_eq!(
        command.revision_id.as_uuid(),
        prepared.revision_id().as_uuid()
    );
    assert_eq!(command.release_id, prepared.release_pin().release_id);
    assert_eq!(
        command.release_agent_id,
        prepared.release_pin().release_agent_id
    );
    assert_eq!(command.project_id, prepared.project_id());
    assert_eq!(&command.name, prepared.name());
    assert_eq!(&command.parameters, prepared.parameters());
    assert_eq!(&command.selected_policy, prepared.selected_policy());
    assert_eq!(&command.platform_policy, prepared.platform_policy());
    assert_eq!(
        command.platform_policy_version,
        prepared.platform_policy_version()
    );
    assert_eq!(command.volumes.len(), 1);
    let selection = &command.volumes[0];
    let frozen = &prepared.volumes()[0];
    assert_eq!(&selection.slot, frozen.slot());
    assert_eq!(&selection.guest_path, frozen.guest_path());
    assert_eq!(selection.access_mode, frozen.access_mode());
    assert_eq!(selection.volume_id, frozen.volume_id());
    assert_eq!(selection.grant_id, frozen.grant_id());
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_effect_attempts),
      (SELECT count(*) FROM recipe_owned_volume_preparations),(SELECT count(*) FROM release_command_inbox),
      (SELECT count(*) FROM agent_instances WHERE id=$1)")
      .bind(prepared.instance_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        counts,
        (0, 0, 0, 0),
        "typed projection is not volume creation, verification or an import"
    );
}
