use crate::LocalWorkspaceManager;
use crate::common::{
    ImportRequest, LocalWorkspaceError, PathBuf, PublishedResult, Run, cas_publish_ref,
    ensure_workspace_path, import_result, join_error, persist_artifacts, repository,
    validate_message, validate_mount_policy,
};
use crate::safety::seal_workspace;

impl LocalWorkspaceManager {
    // The seal, import, database, artifact, and Git CAS order is security
    // sensitive and intentionally visible as one state-machine operation.
    #[allow(clippy::too_many_lines)]
    // Finalization must keep validation and every durable filesystem/database
    // transition in one ordered cleanup path.
    #[allow(clippy::cognitive_complexity)]
    /// Seals, imports, and publishes a completed workspace result.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace lifecycle or persistence operation fails.
    pub(crate) async fn finalize_run(
        &self,
        run: &Run,
        message: &str,
    ) -> Result<Option<PublishedResult>, LocalWorkspaceError> {
        if let Some(result) = self.existing_result(run.id).await? {
            self.cleanup_completed_workspace(run.id).await?;
            return Ok(Some(result));
        }
        let Some(request) = self.request(run).await? else {
            return Ok(None);
        };
        if !request.config.workspace.mount {
            return Ok(None);
        }
        validate_mount_policy(&request.config)?;
        let message = validate_message(message)?;
        let workspace = self
            .metadata
            .workspace(run.id)
            .await
            .map_err(repository)?
            .ok_or_else(|| {
                LocalWorkspaceError::State(String::from("workspace metadata is missing"))
            })?;
        let active = PathBuf::from(&workspace.active_path);
        let sealed = PathBuf::from(&workspace.sealed_path);
        ensure_workspace_path(&self.config, &active, "active")?;
        ensure_workspace_path(&self.config, &sealed, "sealed")?;
        let result_ref = format!("refs/heads/hephaestus/{}/{}", request.instance_id, run.id);
        self.results
            .insert_pending(
                run.id,
                request.repository_id.as_uuid(),
                request.instance_id.as_uuid(),
                run.instance_revision_id.as_uuid(),
                run.release_id.as_uuid(),
                run.release_agent_id.as_uuid(),
                workspace.input_commit.as_deref().unwrap_or_default(),
                &result_ref,
                message,
            )
            .await
            .map_err(repository)?;

        if workspace.state == "active" {
            self.metadata
                .set_state(run.id, "finalize_requested")
                .await
                .map_err(repository)?;
            self.metadata
                .event(
                    run.id,
                    "result.finalize_requested",
                    serde_json::json!({"message": message}),
                )
                .await
                .map_err(repository)?;
        }
        if matches!(
            workspace.state.as_str(),
            "active" | "finalize_requested" | "seal_failed"
        ) {
            if let Err(error) = seal_workspace(&active, &sealed) {
                self.metadata
                    .mark_failed(run.id, "seal_failed", &error.to_string())
                    .await
                    .map_err(repository)?;
                return Err(error);
            }
            self.metadata
                .set_state(run.id, "sealed")
                .await
                .map_err(repository)?;
            self.metadata
                .event(
                    run.id,
                    "result.workspace_sealed",
                    serde_json::json!({"workspace_id": workspace.id}),
                )
                .await
                .map_err(repository)?;
        }

        self.metadata
            .set_state(run.id, "importing")
            .await
            .map_err(repository)?;

        let repository_path = self.repository_path(request.repository_id);
        let config = self.config.clone();
        let sealed_for_import = sealed.clone();
        let input_commit = workspace.input_commit.clone();
        let message_for_import = message.to_owned();
        let declared_files = request.config.results.declared_files.clone();
        let timestamp = run.created_at.unix_timestamp();
        let result_run_id = run.id;
        let import = ImportRequest {
            input_commit: input_commit.unwrap_or_default(),
            message: message_for_import,
            timestamp,
            declared_paths: declared_files,
            repository_id: request.repository_id,
            run_id: result_run_id,
        };
        let imported = tokio::task::spawn_blocking(move || {
            import_result(&config, &repository_path, &sealed_for_import, &import)
        })
        .await
        .map_err(join_error)?;
        let imported = match imported {
            Ok(imported) => imported,
            Err(error) => {
                self.results
                    .reject(run.id, &error.to_string())
                    .await
                    .map_err(repository)?;
                self.metadata
                    .mark_failed(run.id, "import_rejected", &error.to_string())
                    .await
                    .map_err(repository)?;
                self.metadata
                    .event(
                        run.id,
                        "result.import_rejected",
                        serde_json::json!({"message": error.to_string()}),
                    )
                    .await
                    .map_err(repository)?;
                return Err(error);
            }
        };

        let persisted_id = self.results.id_for_run(run.id).await.map_err(repository)?;
        let (logs, exit) = self.results.vm_logs(run.id).await.map_err(repository)?;
        let artifacts = persist_artifacts(
            &imported,
            &logs,
            &exit.unwrap_or(serde_json::Value::Null),
            run.id,
            &self.config.artifact_root,
        )?;
        self.results
            .persist_prepared(
                persisted_id,
                run.id,
                &imported.tree,
                &imported.commit,
                &imported.manifest_hash,
                &artifacts,
            )
            .await
            .map_err(repository)?;
        self.metadata
            .event(
                run.id,
                "result.import_prepared",
                serde_json::json!({
                    "result_id": persisted_id.to_string(),
                    "result_tree": imported.tree,
                    "result_commit": imported.commit,
                    "artifact_manifest_hash": imported.manifest_hash,
                }),
            )
            .await
            .map_err(repository)?;

        cas_publish_ref(
            &self.config,
            &self.repository_path(request.repository_id),
            &result_ref,
            &imported.commit,
        )?;
        self.results
            .mark_ref_published(persisted_id, &imported.commit)
            .await
            .map_err(repository)?;
        self.metadata
            .event(
                run.id,
                "result.ref_published",
                serde_json::json!({"result_ref": result_ref, "result_commit": imported.commit}),
            )
            .await
            .map_err(repository)?;
        self.results
            .mark_completed(persisted_id)
            .await
            .map_err(repository)?;
        self.metadata
            .event(
                run.id,
                "result.completed",
                serde_json::json!({"result_id": persisted_id.to_string()}),
            )
            .await
            .map_err(repository)?;
        self.cleanup_completed_workspace(run.id).await?;
        Ok(Some(PublishedResult {
            id: persisted_id,
            result_ref,
            result_commit: imported.commit,
            result_tree: imported.tree,
        }))
    }
}
