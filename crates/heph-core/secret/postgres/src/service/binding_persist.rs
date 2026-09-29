use super::binding_prepare::BindingPreparation;
use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    #[allow(clippy::too_many_lines)] // Keep revision, binding, and capability writes atomic.
    pub(super) async fn persist_binding_revision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        command: &BindSecret,
        plan: BindingPreparation,
    ) -> Result<(), SecretServiceError> {
        let BindingPreparation {
            revision,
            import,
            copied,
            binding_ids,
            diagnostics,
            runnable,
            carried_capabilities,
            cloned_capabilities,
            publication_repository_binding_id,
        } = plan;
        sqlx::query(
            "INSERT INTO agent_instance_revisions
               (id, instance_id, release_agent_id, parameters, parameter_hash,
                secret_bindings, resource_selection, network_restriction,
                effective_runtime_policy, effective_policy_hash,
                platform_policy_version, publication_mode,
                publication_repository_binding_id, runnable, diagnostics,
                created_by)
               SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                      $11, publication_mode, $12, $13, $14, $15
               FROM agent_instance_revisions WHERE id = $16",
        )
        .bind(command.new_revision_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(revision.release_agent_id)
        .bind(&revision.parameters)
        .bind(&revision.parameter_hash)
        .bind(serde_json::to_value(&binding_ids)?)
        .bind(&revision.resource_selection)
        .bind(&revision.network_restriction)
        .bind(&revision.effective_runtime_policy)
        .bind(&revision.effective_policy_hash)
        .bind(&revision.platform_policy_version)
        .bind(publication_repository_binding_id)
        .bind(runnable)
        .bind(&diagnostics)
        .bind(identity.user_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        for (binding_id, binding) in copied {
            insert_binding_copy(
                tx,
                binding_id,
                command.new_revision_id,
                &binding,
                identity.user_id.as_uuid(),
            )
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        }
        for (binding, cloned) in carried_capabilities.iter().zip(&cloned_capabilities) {
            sqlx::query(
                "INSERT INTO agent_capability_bindings
                   (id, instance_revision_id, release_agent_id, requirement_id,
                    requirement_hash, slot_key, resource_kind, resource_id,
                    granted_operations, normalized_hash,
                    authorization_model_version, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            )
            .bind(cloned.id().as_uuid())
            .bind(command.new_revision_id.as_uuid())
            .bind(revision.release_agent_id)
            .bind(cloned.requirement_id().as_uuid())
            .bind(cloned.requirement_hash().as_bytes().as_slice())
            .bind(cloned.slot().as_str())
            .bind(cloned.resource().kind.as_str())
            .bind(cloned.resource().id)
            .bind(
                cloned
                    .granted_operations()
                    .map(CapabilityOperation::as_str)
                    .collect::<Vec<_>>(),
            )
            .bind(cloned.normalized_hash().as_bytes().as_slice())
            .bind(&binding.authorization_model_version)
            .bind(identity.user_id.as_uuid())
            .execute(&mut **tx)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;

            // Runtime-Git capability bindings are the typed authority paired
            // with the generic capability row. Preserve that immutable
            // authority when a secret binding creates the next revision;
            // the database constraint deliberately rejects a generic Git row
            // without its typed companion.
            sqlx::query(
                "INSERT INTO agent_git_capability_bindings
                   (binding_id, instance_revision_id, requirement_id,
                    grammar_version, git_operations, ref_globs,
                    changed_path_globs, branch_update_policy, branch_create,
                    branch_delete, tag_create, tag_update, tag_delete,
                    other_create, other_update, other_delete, request_bytes,
                    pack_bytes, object_count, ref_updates,
                    exact_parent_required, normalized_hash)
                 SELECT $1, $2, requirement_id, grammar_version,
                        git_operations, ref_globs, changed_path_globs,
                        branch_update_policy, branch_create, branch_delete,
                        tag_create, tag_update, tag_delete, other_create,
                        other_update, other_delete, request_bytes, pack_bytes,
                        object_count, ref_updates, exact_parent_required,
                        normalized_hash
                   FROM agent_git_capability_bindings
                  WHERE binding_id = $3",
            )
            .bind(cloned.id().as_uuid())
            .bind(command.new_revision_id.as_uuid())
            .bind(binding.source_binding_id)
            .execute(&mut **tx)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        }
        let effective_policy = json!({
            "grant_id": import.grant_id,
            "secret_id": import.secret_id,
            "mode": mode_name(command.mode),
            "phases": command.phases.iter().map(|phase| phase_name(*phase)).collect::<Vec<_>>(),
            "attachment_ids": command.attachment_ids,
            "destinations": command.destinations,
        });
        let effective_hash: [u8; 32] =
            sha2::Sha256::digest(serde_json::to_vec(&effective_policy)?).into();
        sqlx::query(
            "INSERT INTO agent_secret_bindings
               (id, instance_revision_id, import_id, slot_key, delivery_mode,
                phases, attachment_ids, destinations, effective_policy,
                effective_policy_hash, status, created_by)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                       'active', $11)",
        )
        .bind(command.binding_id.as_uuid())
        .bind(command.new_revision_id.as_uuid())
        .bind(command.import_id.as_uuid())
        .bind(command.slot.as_str())
        .bind(mode_name(command.mode))
        .bind(
            command
                .phases
                .iter()
                .map(|phase| phase_name(*phase))
                .collect::<Vec<_>>(),
        )
        .bind(&command.attachment_ids)
        .bind(&command.destinations)
        .bind(effective_policy)
        .bind(effective_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let activated = sqlx::query(
            "UPDATE agent_instances
               SET active_revision_id = $3, version = version + 1,
                   updated_at = now()
               WHERE id = $1 AND active_revision_id = $2
                 AND state IN ('active', 'update_rejected')",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.new_revision_id.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        if activated.rows_affected() != 1 {
            return Err(SecretServiceError::StaleInstanceRevision);
        }
        record_command(
            tx,
            command.command_key,
            "bind",
            command.binding_id.as_uuid(),
            Some(command.new_revision_id.as_uuid()),
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        audit(
            tx,
            identity,
            import.owner_organization_id,
            if command.mode == DeliveryMode::Raw {
                "bind_raw"
            } else {
                "bind_brokered"
            },
            if command.mode == DeliveryMode::Raw {
                "secret_import.bind_raw"
            } else {
                "secret_import.bind_brokered"
            },
            Some(SecretId::from_uuid(import.secret_id)),
            None,
            Some(SecretGrantId::from_uuid(import.grant_id)),
            Some(command.import_id),
            "revision_activated",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        Ok(())
    }
}
