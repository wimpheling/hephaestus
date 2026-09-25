use super::Prepared;
use crate::support::{Fixture, SENTINEL, identity, key, seed_attachment};
use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
    CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
};
use release_domain::{AgentInstanceId, AgentInstanceRevisionId};
use secret_application::{AcceptSecretImport, BindSecret, CreateSecret, GrantSecret};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
// This fixture seeds one immutable catalog and revision graph for the authority carry test.
#[allow(clippy::too_many_lines)]
pub(super) async fn prepare(pool: &PgPool, fixture: &Fixture) -> Prepared {
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let source_commit = "b".repeat(40);
    let agent_key = format!("runtime_git_{}", release_agent_id.simple());
    sqlx::query(
        "INSERT INTO build_requests
          (id, repository_id, source_commit, source_ref,
           build_definition_hash, state)
          VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(fixture.target_repository.as_uuid())
    .bind(&source_commit)
    .bind([11_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed runtime Git release build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
          VALUES ($1, $2, $3)",
    )
    .bind(family_id)
    .bind(fixture.target_repository.as_uuid())
    .bind(&agent_key)
    .execute(pool)
    .await
    .expect("seed runtime Git agent family");
    sqlx::query(
        "INSERT INTO releases
          (id, repository_id, version, source_commit, source_ref,
           build_request_id, build_definition_hash, configuration,
           configuration_hash, manifest_hash, state)
          VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6,
                  '{}', $7, $8, 'draft')",
    )
    .bind(release_id)
    .bind(fixture.target_repository.as_uuid())
    .bind(format!("runtime-git-{}", release_id.simple()))
    .bind(&source_commit)
    .bind(build_id)
    .bind([11_u8; 32].as_slice())
    .bind([12_u8; 32].as_slice())
    .bind([13_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed runtime Git draft release");
    let mut tx = pool
        .begin()
        .await
        .expect("begin runtime Git release fixture");
    sqlx::query(
        "INSERT INTO release_agents
          (id, release_id, family_id, agent_key, display_name,
           runtime_contract, runtime_contract_hash, parameter_schema,
           secret_slot_schema, requires_state, publication_mode,
           publication_repository_slot)
          VALUES ($1, $2, $3, $4, 'Runtime Git fixture', $5, $6, '[]', $7,
                  false, 'runtime_git', 'session')",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(&agent_key)
    .bind(json!({
        "policy_ceiling": {
            "vcpus": 2,
            "memory_mib": 1024,
            "network": "broker_only"
        }
    }))
    .bind([14_u8; 32].as_slice())
    .bind(json!([{
        "key": "model",
        "purpose": "Call model",
        "required": true,
        "delivery_modes": ["brokered"],
        "phases": ["normal"],
        "destinations": ["api.example.test"]
    }]))
    .execute(&mut *tx)
    .await
    .expect("seed runtime Git release agent");
    let instance_id = AgentInstanceId::new();
    let initial_revision_id = AgentInstanceRevisionId::new();
    let attachment_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_instances
          (id, project_id, family_id, name, state, active_revision_id,
           created_by)
          VALUES ($1, $2, $3, $4, 'active', NULL, $5)",
    )
    .bind(instance_id.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(family_id)
    .bind(format!("runtime_git_{}", instance_id.as_uuid().simple()))
    .bind(fixture.target_manager.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed runtime Git instance");
    let requirement_id = Uuid::new_v4();
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(requirement_id),
        CapabilitySlotKey::parse("session").expect("session capability slot"),
        CapabilityResourceKind::Repository,
        [CapabilityOperation::GitRead, CapabilityOperation::UpdateRef],
        [],
        true,
    )
    .expect("runtime Git capability requirement");
    let requirement_hash = requirement.normalized_hash();
    let source_binding_id = CapabilityBindingId::new();
    let source_binding = CapabilityBinding::bind(
        source_binding_id,
        &requirement,
        CapabilityResource::new(
            CapabilityResourceKind::Repository,
            fixture.target_repository.as_uuid(),
        ),
        [CapabilityOperation::GitRead, CapabilityOperation::UpdateRef],
    )
    .expect("runtime Git capability binding");
    let source_binding_hash = source_binding.normalized_hash();
    // This fresh draft release is made runtime-Git capable through the same
    // immutable catalog rows that a published release carries.
    sqlx::query(
        "INSERT INTO release_capability_requirements
           (id, release_agent_id, slot_key, purpose, resource_kind,
            required_operations, optional_operations, slot_required,
            normalized_hash)
         VALUES ($1, $2, 'session', 'Session repository', 'repository',
                 ARRAY['git_read', 'update_ref'], ARRAY[]::text[], true, $3)",
    )
    .bind(requirement_id)
    .bind(release_agent_id)
    .bind(requirement_hash.as_bytes().as_slice())
    .execute(&mut *tx)
    .await
    .expect("insert runtime Git capability requirement");
    sqlx::query(
        "INSERT INTO release_git_capability_ceilings
           (requirement_id, release_agent_id, grammar_version, git_operations,
            ref_globs, changed_path_globs, branch_update_policy, branch_create,
            branch_delete, tag_create, tag_update, tag_delete, other_create,
            other_update, other_delete, request_bytes, pack_bytes, object_count,
            ref_updates, exact_parent_required, normalized_hash)
         VALUES ($1, $2, 1, ARRAY['discover', 'fetch', 'receive'],
                 ARRAY['refs/heads/main'], ARRAY['.heph/session/**'],
                 'fast_forward_only', false, false, false, false, false, false,
                 false, false, 1048576, 8388608, 10000, 8, true, $3)",
    )
    .bind(requirement_id)
    .bind(release_agent_id)
    .bind([31_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("insert runtime Git capability ceiling");
    sqlx::query("UPDATE releases SET state = 'published', published_at = now() WHERE id = $1")
        .bind(release_id)
        .execute(&mut *tx)
        .await
        .expect("publish runtime Git fixture");
    tx.commit()
        .await
        .expect("commit runtime Git release fixture");
    seed_attachment(pool, fixture, instance_id, attachment_id).await;
    sqlx::query(
        "WITH inserted_revision AS (
             INSERT INTO agent_instance_revisions
               (id, instance_id, release_agent_id, parameters, parameter_hash,
                secret_bindings, resource_selection, network_restriction,
                effective_runtime_policy, effective_policy_hash,
                platform_policy_version, publication_mode,
                publication_repository_binding_id, runnable, diagnostics,
                created_by)
             VALUES ($1, $2, $3, '{}', $4, '[]', $5, $6, $5, $7,
                     'platform/v1', 'runtime_git', $8, false, $9, $10)
             RETURNING id
         ), inserted_binding AS (
             INSERT INTO agent_capability_bindings
               (id, instance_revision_id, release_agent_id, requirement_id,
                requirement_hash, slot_key, resource_kind, resource_id,
                granted_operations, normalized_hash, authorization_model_version,
                created_by)
             SELECT $8, id, $3, $11, $12, 'session', 'repository', $13,
                    ARRAY['git_read', 'update_ref'], $14, 'authz/v1', $10
             FROM inserted_revision
             RETURNING id, instance_revision_id
         )
         INSERT INTO agent_git_capability_bindings
           (binding_id, instance_revision_id, requirement_id, grammar_version,
            git_operations, ref_globs, changed_path_globs,
            branch_update_policy, branch_create, branch_delete, tag_create,
            tag_update, tag_delete, other_create, other_update, other_delete,
            request_bytes, pack_bytes, object_count, ref_updates,
            exact_parent_required, normalized_hash)
         SELECT id, instance_revision_id, $11, 1,
                ARRAY['discover', 'fetch', 'receive'],
                ARRAY['refs/heads/main'], ARRAY['.heph/session/**'],
                'fast_forward_only', false, false, false, false, false,
                false, false, false, 1048576, 8388608, 10000, 8, true, $15
         FROM inserted_binding",
    )
    .bind(initial_revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 512, "network": "broker_only"}))
    .bind(json!({"network": "broker_only"}))
    .bind([6_u8; 32].as_slice())
    .bind(source_binding_id.as_uuid())
    .bind(json!([{
        "code": "required_secret_binding_missing",
        "field": "secret_slots.model"
    }]))
    .bind(fixture.target_manager.as_uuid())
    .bind(requirement_id)
    .bind(requirement_hash.as_bytes().as_slice())
    .bind(fixture.target_repository.as_uuid())
    .bind(source_binding_hash.as_bytes().as_slice())
    .bind([31_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed runtime Git revision and typed authority");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(initial_revision_id.as_uuid())
        .execute(pool)
        .await
        .expect("activate runtime Git revision");
    let secret_service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("test/v1", [("test/v1", [7_u8; 32])])
                .expect("fixture key should validate"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let owner_identity = identity(fixture.owner);
    let identity = identity(fixture.target_manager);
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    secret_service
        .create(
            &owner_identity,
            CreateSecret {
                command_key: key("git-carry-secret", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(fixture.organization),
                name: SecretName::parse(format!("git_carry_{secret_id}"))
                    .expect("fixture secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(SENTINEL).expect("fixture secret value"),
            },
        )
        .await
        .expect("create carry-forward secret");
    let import_id = SecretImportId::new();
    let grant_id = SecretGrantId::new();
    secret_service
        .grant(
            &owner_identity,
            GrantSecret {
                command_key: key("git-carry-grant", grant_id.as_uuid()),
                grant_id,
                secret_id,
                target: SecretTarget::Project(fixture.target_project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
            },
        )
        .await
        .expect("grant carry-forward secret");
    secret_service
        .accept_import(
            &identity,
            AcceptSecretImport {
                command_key: key("git-carry-import", import_id.as_uuid()),
                import_id,
                grant_id,
                target: SecretTarget::Project(fixture.target_project),
                alias: SecretAlias::parse("model").expect("fixture secret alias"),
            },
        )
        .await
        .expect("accept carry-forward secret");
    let new_revision_id = AgentInstanceRevisionId::new();
    let new_secret_binding_id = AgentSecretBindingId::new();
    secret_service
        .bind_secret(
            &identity,
            BindSecret {
                command_key: key("git-carry-bind", new_revision_id.as_uuid()),
                binding_id: new_secret_binding_id,
                instance_id,
                expected_revision_id: initial_revision_id,
                new_revision_id,
                import_id,
                slot: SecretSlotKey::parse("model").expect("model slot"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment_id],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect("bind secret while carrying typed Git authority");
    Prepared {
        instance_id,
        initial_revision_id,
        source_binding_id,
        requirement,
        source_binding_hash,
        new_revision_id,
    }
}
