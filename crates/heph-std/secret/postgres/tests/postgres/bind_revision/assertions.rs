use super::Prepared;
use crate::support::{CarriedTypedGitAuthority, Fixture, TypedGitAuthority};
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityResource,
    CapabilityResourceKind,
};
use sqlx::PgPool;
use uuid::Uuid;

// Keep the SQL evidence assertions together so the test covers every copied authority field.
#[allow(clippy::too_many_lines)]
pub(super) async fn assert_carried(pool: &PgPool, fixture: &Fixture, prepared: &Prepared) {
    let instance_id = prepared.instance_id;
    let initial_revision_id = prepared.initial_revision_id;
    let source_binding_id = prepared.source_binding_id;
    let requirement = &prepared.requirement;
    let source_binding_hash = &prepared.source_binding_hash;
    let new_revision_id = prepared.new_revision_id;

    let source_typed: TypedGitAuthority = sqlx::query_as(
        "SELECT git_operations, ref_globs, changed_path_globs, request_bytes,
                    pack_bytes, object_count, ref_updates, exact_parent_required,
                    normalized_hash
               FROM agent_git_capability_bindings
              WHERE binding_id = $1 AND instance_revision_id = $2",
    )
    .bind(source_binding_id.as_uuid())
    .bind(initial_revision_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("source typed Git authority");
    let carried_typed: CarriedTypedGitAuthority = sqlx::query_as(
        "SELECT binding_id, git_operations, ref_globs, changed_path_globs,
                    request_bytes, pack_bytes, object_count, ref_updates,
                    exact_parent_required, normalized_hash
               FROM agent_git_capability_bindings
              WHERE instance_revision_id = $1",
    )
    .bind(new_revision_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("carried typed Git authority");
    assert_ne!(carried_typed.0, source_binding_id.as_uuid());
    assert_eq!(
        (
            carried_typed.1,
            carried_typed.2,
            carried_typed.3,
            carried_typed.4,
            carried_typed.5,
            carried_typed.6,
            carried_typed.7,
            carried_typed.8,
            carried_typed.9,
        ),
        source_typed,
    );
    let source_generic_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT normalized_hash FROM agent_capability_bindings
           WHERE id = $1 AND instance_revision_id = $2",
    )
    .bind(source_binding_id.as_uuid())
    .bind(initial_revision_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("source generic capability hash");
    let carried_generic_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT normalized_hash FROM agent_capability_bindings
           WHERE id = $1 AND instance_revision_id = $2",
    )
    .bind(carried_typed.0)
    .bind(new_revision_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("carried generic capability hash");
    let carried_binding = CapabilityBinding::bind(
        CapabilityBindingId::from_uuid(carried_typed.0),
        requirement,
        CapabilityResource::new(
            CapabilityResourceKind::Repository,
            fixture.target_repository.as_uuid(),
        ),
        [CapabilityOperation::GitRead, CapabilityOperation::UpdateRef],
    )
    .expect("recompute carried runtime Git capability binding");
    assert_eq!(source_generic_hash, source_binding_hash.as_bytes());
    assert_eq!(
        carried_generic_hash,
        carried_binding.normalized_hash().as_bytes()
    );
    assert_ne!(source_generic_hash, carried_generic_hash);
    let (new_publication_binding, new_secret_revision): (Uuid, Uuid) = sqlx::query_as(
        "SELECT revision.publication_repository_binding_id, instance.active_revision_id
           FROM agent_instances AS instance
           JOIN agent_instance_revisions AS revision
             ON revision.id = instance.active_revision_id
          WHERE instance.id = $1",
    )
    .bind(instance_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("new active publication binding");
    assert_eq!(new_secret_revision, new_revision_id.as_uuid());
    assert_eq!(new_publication_binding, carried_typed.0);
    let old_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_git_capability_bindings
          WHERE binding_id = $1 AND instance_revision_id = $2",
    )
    .bind(source_binding_id.as_uuid())
    .bind(initial_revision_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("historical typed Git authority");
    assert_eq!(old_count, 1);
}
