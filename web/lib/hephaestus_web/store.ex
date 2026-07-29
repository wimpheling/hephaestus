defmodule HephaestusWeb.Store do
  @moduledoc """
  RLS-aware read models and durable browser command creation.

  Every operation begins a transaction and installs transaction-local actor
  and request context before touching protected tables.
  """

  alias HephaestusWeb.{Identity, Repo}

  def list_organizations(%Identity{} = identity) do
    with_actor(identity, fn ->
      query_maps("""
      SELECT organization.id, organization.name,
             count(DISTINCT project.id)::bigint AS project_count,
             count(DISTINCT repository.id)::bigint AS repository_count
      FROM organizations organization
      LEFT JOIN projects project ON project.organization_id = organization.id
      LEFT JOIN repositories repository ON repository.project_id = project.id
      GROUP BY organization.id, organization.name
      ORDER BY organization.name
      """)
    end)
  end

  def list_repositories(%Identity{} = identity, organization_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT repository.id, repository.name, repository.default_branch,
               repository.is_public, project.name AS project_name,
               count(run.id)::bigint AS run_count,
               max(run.created_at) AS last_run_at
        FROM repositories repository
        JOIN projects project ON project.id = repository.project_id
        LEFT JOIN run_requests request ON request.repository_id = repository.id
        LEFT JOIN runs run ON run.id = request.run_id
        WHERE project.organization_id = $1
        GROUP BY repository.id, repository.name, repository.default_branch,
                 repository.is_public, project.name
        ORDER BY project.name, repository.name
        """,
        [uuid!(organization_id)]
      )
    end)
  end

  def list_projects(%Identity{} = identity, organization_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT project.id, project.name,
               count(DISTINCT repository.id)::bigint AS repository_count,
               count(DISTINCT instance.id)::bigint AS instance_count,
               count(DISTINCT run.id)::bigint AS run_count,
               max(run.updated_at) AS last_activity_at
        FROM projects project
        LEFT JOIN repositories repository ON repository.project_id = project.id
        LEFT JOIN agent_instances instance ON instance.project_id = project.id
        LEFT JOIN runs run ON run.instance_id = instance.id
        WHERE project.organization_id = $1
        GROUP BY project.id, project.name
        ORDER BY project.name
        """,
        [uuid!(organization_id)]
      )
    end)
  end

  def get_organization(%Identity{} = identity, organization_id) do
    with_actor(identity, fn ->
      case query_one(
             "SELECT id, name FROM organizations WHERE id = $1",
             [uuid!(organization_id)]
           ) do
        nil -> Repo.rollback(:forbidden)
        organization -> organization
      end
    end)
  end

  def get_project(%Identity{} = identity, project_id) do
    with_actor(identity, fn ->
      authorized =
        Ecto.Adapters.SQL.query!(
          Repo,
          "SELECT check_permission('user', $1, 'can_read', 'project', $2) = 1",
          [identity.user_id, project_id]
        ).rows == [[true]]

      unless authorized, do: Repo.rollback(:forbidden)

      case query_one(
             """
             SELECT project.id, project.name,
                    organization.id AS organization_id,
                    organization.name AS organization_name
             FROM projects project
             JOIN organizations organization
               ON organization.id = project.organization_id
             WHERE project.id = $1
             """,
             [uuid!(project_id)]
           ) do
        nil -> Repo.rollback(:forbidden)
        project -> project
      end
    end)
  end

  def list_project_repositories(%Identity{} = identity, project_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT repository.id, repository.name, repository.default_branch,
               repository.is_public,
               count(DISTINCT attachment.id)::bigint AS attachment_count,
               count(DISTINCT request.run_id)::bigint AS run_count
        FROM repositories repository
        LEFT JOIN agent_attachments attachment
          ON attachment.repository_id = repository.id
         AND attachment.removed_at IS NULL
        LEFT JOIN run_requests request ON request.repository_id = repository.id
        WHERE repository.project_id = $1
        GROUP BY repository.id, repository.name, repository.default_branch,
                 repository.is_public
        ORDER BY repository.name
        """,
        [uuid!(project_id)]
      )
    end)
  end

  def list_project_instances(%Identity{} = identity, project_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT instance.id, instance.name, instance.state,
               instance.run_gate_open, instance.active_revision_id,
               instance.state_volume_id, instance.updated_at,
               revision.runnable, revision.platform_policy_version,
               revision.diagnostics,
               release.id AS release_id, release.version AS release_version,
               release.state AS release_state,
               release_agent.display_name AS release_agent_name,
               count(DISTINCT attachment.id)::bigint AS attachment_count,
               count(DISTINCT run.id)::bigint AS run_count,
               max(run.updated_at) AS last_run_at
        FROM agent_instances instance
        LEFT JOIN agent_instance_revisions revision
          ON revision.id = instance.active_revision_id
        LEFT JOIN release_agents release_agent
          ON release_agent.id = revision.release_agent_id
        LEFT JOIN releases release ON release.id = release_agent.release_id
        LEFT JOIN agent_attachments attachment
          ON attachment.instance_id = instance.id
         AND attachment.removed_at IS NULL
        LEFT JOIN runs run ON run.instance_id = instance.id
        WHERE instance.project_id = $1
        GROUP BY instance.id, instance.name, instance.state,
                 instance.run_gate_open, instance.active_revision_id,
                 instance.state_volume_id, instance.updated_at,
                 revision.runnable, revision.platform_policy_version,
                 revision.diagnostics, release.id, release.version,
                 release.state, release_agent.display_name
        ORDER BY instance.name
        """,
        [uuid!(project_id)]
      )
    end)
  end

  def list_importable_release_agents(%Identity{} = identity, project_id) do
    with_actor(identity, fn ->
      project_manageable =
        Ecto.Adapters.SQL.query!(
          Repo,
          "SELECT check_permission('user', $1, 'can_manage', 'project', $2) = 1",
          [identity.user_id, project_id]
        ).rows == [[true]]

      if project_manageable do
        query_maps("""
        SELECT release_agent.id, release_agent.display_name,
               release_agent.parameter_schema, release_agent.secret_slot_schema,
               release_agent.runtime_contract, release_agent.requires_state,
               release.id AS release_id, release.version AS release_version,
               release.source_commit, repository.id AS repository_id,
               repository.name AS repository_name
        FROM release_agents release_agent
        JOIN releases release ON release.id = release_agent.release_id
        JOIN repositories repository ON repository.id = release.repository_id
        WHERE release.state = 'published'
          AND check_permission(
            'user', hephaestus_actor_id(), 'can_use',
            'release_agent', release_agent.id::text
          ) = 1
        ORDER BY repository.name, release_agent.display_name, release.version
        """)
      else
        []
      end
    end)
  end

  def list_project_runs(%Identity{} = identity, project_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT run.id, run.state, run.outcome, run.run_kind, run.updated_at,
               instance.id AS instance_id, instance.name AS instance_name,
               request.repository_id, repository.name AS repository_name,
               request.commit_sha, request.git_ref,
               release.id AS release_id, release.version AS release_version,
               run.instance_revision_id
        FROM runs run
        JOIN agent_instances instance ON instance.id = run.instance_id
        LEFT JOIN run_requests request ON request.run_id = run.id
        LEFT JOIN repositories repository ON repository.id = request.repository_id
        JOIN releases release ON release.id = run.release_id
        WHERE instance.project_id = $1
        ORDER BY run.created_at DESC
        LIMIT 200
        """,
        [uuid!(project_id)]
      )
    end)
  end

  def list_project_secrets(%Identity{} = identity, project_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT secret.id, secret.name, secret.status,
               secret.allowed_delivery_modes, secret.active_version_id,
               secret.created_at, secret.updated_at,
               version.sequence AS active_version_sequence,
               version.created_at AS active_version_created_at,
               count(DISTINCT secret_grant.id)::bigint AS grant_count,
               count(DISTINCT secret_import.id)::bigint AS import_count,
               count(DISTINCT binding.id)::bigint AS binding_count,
               bool_or(binding.delivery_mode = 'raw') AS has_raw_binding,
               check_permission(
                 'user', hephaestus_actor_id(), 'rotate',
                 'secret', secret.id::text
               ) = 1 AS can_rotate,
               check_permission(
                 'user', hephaestus_actor_id(), 'manage_grants',
                 'secret', secret.id::text
               ) = 1 AS can_manage_grants,
               check_permission(
                 'user', hephaestus_actor_id(), 'revoke',
                 'secret', secret.id::text
               ) = 1 AS can_revoke,
               check_permission(
                 'user', hephaestus_actor_id(), 'purge',
                 'secret', secret.id::text
               ) = 1 AS can_purge,
               (
                 SELECT jsonb_build_object(
                   'operation', audit.operation,
                   'outcome', audit.outcome,
                   'delivery_mode', audit.delivery_mode,
                   'occurred_at', audit.occurred_at
                 )
                 FROM secret_audit_events audit
                 WHERE audit.secret_id = secret.id
                 ORDER BY audit.occurred_at DESC LIMIT 1
               ) AS last_use
        FROM secrets secret
        LEFT JOIN secret_version_metadata version
          ON version.id = secret.active_version_id
        LEFT JOIN secret_grants secret_grant ON secret_grant.secret_id = secret.id
        LEFT JOIN secret_imports secret_import ON secret_import.secret_id = secret.id
        LEFT JOIN agent_secret_bindings binding ON binding.import_id = secret_import.id
        WHERE secret.project_id = $1
        GROUP BY secret.id, secret.name, secret.status,
                 secret.allowed_delivery_modes, secret.active_version_id,
                 secret.created_at, secret.updated_at,
                 version.sequence, version.created_at
        ORDER BY secret.name
        """,
        [uuid!(project_id)]
      )
    end)
  end

  def list_organization_secrets(%Identity{} = identity, organization_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT secret.id, secret.name, secret.status,
               secret.allowed_delivery_modes, secret.active_version_id,
               version.sequence AS active_version_sequence,
               version.created_at AS active_version_created_at,
               count(DISTINCT secret_grant.id)::bigint AS grant_count,
               count(DISTINCT secret_import.id)::bigint AS import_count,
               count(DISTINCT binding.id)::bigint AS binding_count,
               bool_or(binding.delivery_mode = 'raw') AS has_raw_binding,
               check_permission(
                 'user', hephaestus_actor_id(), 'rotate',
                 'secret', secret.id::text
               ) = 1 AS can_rotate,
               check_permission(
                 'user', hephaestus_actor_id(), 'manage_grants',
                 'secret', secret.id::text
               ) = 1 AS can_manage_grants,
               check_permission(
                 'user', hephaestus_actor_id(), 'revoke',
                 'secret', secret.id::text
               ) = 1 AS can_revoke,
               check_permission(
                 'user', hephaestus_actor_id(), 'purge',
                 'secret', secret.id::text
               ) = 1 AS can_purge
        FROM secrets secret
        LEFT JOIN secret_version_metadata version ON version.id = secret.active_version_id
        LEFT JOIN secret_grants secret_grant ON secret_grant.secret_id = secret.id
        LEFT JOIN secret_imports secret_import ON secret_import.secret_id = secret.id
        LEFT JOIN agent_secret_bindings binding ON binding.import_id = secret_import.id
        WHERE secret.organization_id = $1
        GROUP BY secret.id, secret.name, secret.status,
                 secret.allowed_delivery_modes, secret.active_version_id,
                 version.sequence, version.created_at
        ORDER BY secret.name
        """,
        [uuid!(organization_id)]
      )
    end)
  end

  def list_organization_secret_grants(%Identity{} = identity, organization_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT secret_grant.id, secret_grant.secret_id,
               secret.name AS secret_name, secret_grant.target_kind,
               secret_grant.target_id,
               CASE secret_grant.target_kind
                 WHEN 'project' THEN target_project.name
                 WHEN 'repository' THEN
                   repository_project.name || '/' || target_repository.name
               END AS target_name,
               secret_grant.delivery_modes, secret_grant.phases,
               secret_grant.destinations, secret_grant.expires_at,
               secret_grant.status, secret_grant.created_at,
               count(DISTINCT secret_import.id)::bigint AS import_count
        FROM secret_grants secret_grant
        JOIN secrets secret ON secret.id = secret_grant.secret_id
        LEFT JOIN projects target_project
          ON secret_grant.target_kind = 'project'
         AND target_project.id = secret_grant.target_id
        LEFT JOIN repositories target_repository
          ON secret_grant.target_kind = 'repository'
         AND target_repository.id = secret_grant.target_id
        LEFT JOIN projects repository_project
          ON repository_project.id = target_repository.project_id
        LEFT JOIN secret_imports secret_import
          ON secret_import.grant_id = secret_grant.id
        WHERE secret.organization_id = $1
        GROUP BY secret_grant.id, secret_grant.secret_id, secret.name,
                 secret_grant.target_kind, secret_grant.target_id,
                 target_project.name, target_repository.name,
                 repository_project.name, secret_grant.delivery_modes,
                 secret_grant.phases, secret_grant.destinations,
                 secret_grant.expires_at, secret_grant.status,
                 secret_grant.created_at
        ORDER BY secret.name, secret_grant.created_at DESC
        """,
        [uuid!(organization_id)]
      )
    end)
  end

  def list_project_secret_authority(%Identity{} = identity, project_id) do
    with_actor(identity, fn ->
      grants =
        query_maps(
          """
          SELECT secret_grant.id, secret_grant.secret_id,
                 secret.name AS secret_name, secret_grant.target_kind,
                 secret_grant.target_id, secret_grant.delivery_modes,
                 secret_grant.phases, secret_grant.destinations,
                 secret_grant.expires_at, secret_grant.status,
                 secret_import.id AS import_id, secret_import.alias,
                 secret_import.status AS import_status
          FROM secret_grants secret_grant
          JOIN secrets secret ON secret.id = secret_grant.secret_id
          LEFT JOIN secret_imports secret_import
            ON secret_import.grant_id = secret_grant.id
           AND secret_import.target_kind = secret_grant.target_kind
           AND secret_import.target_id = secret_grant.target_id
          WHERE (
              secret_grant.target_kind = 'project'
              AND secret_grant.target_id = $1
            )
            OR (
              secret_grant.target_kind = 'repository'
              AND secret_grant.target_id IN (
                SELECT id FROM repositories WHERE project_id = $1
              )
            )
          ORDER BY secret.name, secret_grant.created_at
          """,
          [uuid!(project_id)]
        )

      imports =
        query_maps(
          """
          SELECT secret_import.id, secret_import.alias,
                 secret_import.target_kind, secret_import.target_id,
                 secret_import.status, secret.id AS secret_id,
                 secret.name AS secret_name, secret.status AS secret_status,
                 secret_grant.delivery_modes, secret_grant.phases,
                 secret_grant.destinations, secret_grant.expires_at
          FROM secret_imports secret_import
          JOIN secret_grants secret_grant ON secret_grant.id = secret_import.grant_id
          JOIN secrets secret ON secret.id = secret_import.secret_id
          WHERE (
              secret_import.target_kind = 'project'
              AND secret_import.target_id = $1
            )
            OR (
              secret_import.target_kind = 'repository'
              AND secret_import.target_id IN (
                SELECT id FROM repositories WHERE project_id = $1
              )
            )
          ORDER BY secret_import.alias, secret_import.id
          """,
          [uuid!(project_id)]
        )

      %{"grants" => grants, "imports" => imports}
    end)
  end

  def get_repository(%Identity{} = identity, repository_id) do
    with_actor(identity, fn ->
      authorized =
        Ecto.Adapters.SQL.query!(
          Repo,
          "SELECT check_permission('user', $1, 'can_read', 'repository', $2) = 1",
          [identity.user_id, repository_id]
        ).rows == [[true]]

      unless authorized, do: Repo.rollback(:forbidden)

      repository =
        query_one(
          """
          SELECT repository.id, repository.name, repository.default_branch,
                 repository.is_public, project.id AS project_id,
                 project.name AS project_name, organization.id AS organization_id,
                 organization.name AS organization_name
          FROM repositories repository
          JOIN projects project ON project.id = repository.project_id
          JOIN organizations organization ON organization.id = project.organization_id
          WHERE repository.id = $1
          """,
          [uuid!(repository_id)]
        )

      case repository do
        nil ->
          Repo.rollback(:forbidden)

        repository ->
          runs =
            query_maps(
              """
              SELECT run.id, run.state, run.outcome, run.exit_code, run.failure,
                     run.created_at, run.updated_at,
                     instance.name AS agent_name,
                     request.commit_sha, request.git_ref, request.attempt,
                     proposal.id AS proposal_id, proposal.state AS proposal_state
              FROM run_requests request
              JOIN runs run ON run.id = request.run_id
              JOIN agent_instances instance ON instance.id = run.instance_id
              LEFT JOIN review_proposals proposal ON proposal.run_id = run.id
              WHERE request.repository_id = $1
              ORDER BY run.created_at DESC
              LIMIT 100
              """,
              [uuid!(repository_id)]
            )

          Map.put(repository, "runs", runs)
      end
    end)
  end

  def list_repository_releases(%Identity{} = identity, repository_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT release.id, release.version, release.state,
               release.source_commit, release.source_ref,
               release.build_request_id, release.created_at,
               release.published_at, release.manifest_hash,
               count(DISTINCT artifact.id)::bigint AS artifact_count,
               count(DISTINCT release_agent.id)::bigint AS exported_agent_count
        FROM releases release
        LEFT JOIN release_artifacts artifact ON artifact.release_id = release.id
        LEFT JOIN release_agents release_agent ON release_agent.release_id = release.id
        WHERE release.repository_id = $1
        GROUP BY release.id, release.version, release.state,
                 release.source_commit, release.source_ref,
                 release.build_request_id, release.created_at,
                 release.published_at, release.manifest_hash
        ORDER BY release.created_at DESC
        """,
        [uuid!(repository_id)]
      )
    end)
  end

  def get_release(%Identity{} = identity, release_id) do
    with_actor(identity, fn ->
      authorized =
        Ecto.Adapters.SQL.query!(
          Repo,
          "SELECT check_permission('user', $1, 'can_read', 'release', $2) = 1",
          [identity.user_id, release_id]
        ).rows == [[true]]

      unless authorized, do: Repo.rollback(:forbidden)

      release =
        query_one(
          """
          SELECT release.id, release.version, release.state,
                 release.source_commit, release.source_ref,
                 release.build_request_id, release.build_definition_hash,
                 release.configuration_hash, release.manifest_hash,
                 release.created_at, release.published_at, release.revoked_at,
                 repository.id AS repository_id,
                 repository.name AS repository_name,
                 project.id AS project_id, project.name AS project_name,
                 organization.id AS organization_id,
                 organization.name AS organization_name,
                 execution.state AS build_state,
                 execution.exit_code AS build_exit_code,
                 execution.failure_code AS build_failure_code,
                 execution.logs AS build_logs,
                 execution.metrics AS build_metrics
          FROM releases release
          JOIN repositories repository ON repository.id = release.repository_id
          JOIN projects project ON project.id = repository.project_id
          JOIN organizations organization
            ON organization.id = project.organization_id
          LEFT JOIN build_executions execution
            ON execution.build_request_id = release.build_request_id
          WHERE release.id = $1
          """,
          [uuid!(release_id)]
        )

      if release == nil, do: Repo.rollback(:forbidden)

      artifacts =
        query_maps(
          """
          SELECT id, path, kind, mode, content_hash, size_bytes,
                 media_type, storage_key, provenance
          FROM release_artifacts
          WHERE release_id = $1
          ORDER BY path, id
          """,
          [uuid!(release_id)]
        )

      agents =
        query_maps(
          """
          SELECT id, family_id, agent_key, display_name, runtime_contract,
                 parameter_schema, secret_slot_schema, requires_state,
                 update_hook, created_at
          FROM release_agents
          WHERE release_id = $1
          ORDER BY agent_key, id
          """,
          [uuid!(release_id)]
        )

      release
      |> Map.put("artifacts", artifacts)
      |> Map.put("agents", agents)
    end)
  end

  def list_repository_instances(%Identity{} = identity, repository_id) do
    with_actor(identity, fn ->
      query_maps(
        """
        SELECT attachment.id, attachment.ref_selector,
               attachment.trigger_policy, attachment.enabled,
               attachment.removed_at,
               instance.id AS instance_id, instance.name AS instance_name,
               instance.state AS instance_state,
               project.id AS project_id, project.name AS project_name,
               release.id AS release_id, release.version AS release_version
        FROM agent_attachments attachment
        JOIN agent_instances instance ON instance.id = attachment.instance_id
        JOIN projects project ON project.id = instance.project_id
        JOIN agent_instance_revisions revision
          ON revision.id = instance.active_revision_id
        JOIN release_agents release_agent
          ON release_agent.id = revision.release_agent_id
        JOIN releases release ON release.id = release_agent.release_id
        WHERE attachment.repository_id = $1
        ORDER BY project.name, instance.name, attachment.ref_selector
        """,
        [uuid!(repository_id)]
      )
    end)
  end

  def get_instance(%Identity{} = identity, instance_id) do
    with_actor(identity, fn ->
      authorized =
        Ecto.Adapters.SQL.query!(
          Repo,
          "SELECT check_permission('user', $1, 'can_read', 'agent_instance', $2) = 1",
          [identity.user_id, instance_id]
        ).rows == [[true]]

      unless authorized, do: Repo.rollback(:forbidden)

      instance =
        query_one(
          """
          SELECT instance.id, instance.name, instance.state,
                 instance.run_gate_open, instance.active_revision_id,
                 instance.state_volume_id, instance.created_at,
                 instance.updated_at,
                 project.id AS project_id, project.name AS project_name,
                 organization.id AS organization_id,
                 organization.name AS organization_name,
                 check_permission(
                   'user', hephaestus_actor_id(), 'can_manage',
                   'agent_instance', instance.id::text
                 ) = 1 AS can_manage,
                 check_permission(
                   'user', hephaestus_actor_id(), 'can_update',
                   'agent_instance', instance.id::text
                 ) = 1 AS can_update,
                 check_permission(
                   'user', hephaestus_actor_id(), 'can_recover',
                   'agent_instance', instance.id::text
                 ) = 1 AS can_recover
          FROM agent_instances instance
          JOIN projects project ON project.id = instance.project_id
          JOIN organizations organization
            ON organization.id = project.organization_id
          WHERE instance.id = $1
          """,
          [uuid!(instance_id)]
        )

      if instance == nil, do: Repo.rollback(:forbidden)

      revisions =
        query_maps(
          """
          SELECT revision.id, revision.parameters, revision.parameter_hash,
                 revision.resource_selection, revision.network_restriction,
                 revision.effective_runtime_policy,
                 revision.platform_policy_version, revision.runnable,
                 revision.diagnostics, revision.created_at,
                 release_agent.id AS release_agent_id,
                 release_agent.parameter_schema,
                 release_agent.secret_slot_schema,
                 release_agent.runtime_contract,
                 release_agent.update_hook,
                 release.id AS release_id, release.version AS release_version,
                 release.state AS release_state,
                 release_agent.display_name AS release_agent_name
          FROM agent_instance_revisions revision
          JOIN release_agents release_agent
            ON release_agent.id = revision.release_agent_id
          JOIN releases release ON release.id = release_agent.release_id
          WHERE revision.instance_id = $1
          ORDER BY revision.created_at DESC, revision.id
          """,
          [uuid!(instance_id)]
        )

      attachments =
        query_maps(
          """
          SELECT attachment.id, attachment.ref_selector,
                 attachment.trigger_policy, attachment.enabled,
                 attachment.removed_at, repository.id AS repository_id,
                 repository.name AS repository_name,
                 check_permission(
                   'user', hephaestus_actor_id(), 'can_manage',
                   'agent_attachment', attachment.id::text
                 ) = 1 AS can_manage
          FROM agent_attachments attachment
          JOIN repositories repository ON repository.id = attachment.repository_id
          WHERE attachment.instance_id = $1
          ORDER BY repository.name, attachment.ref_selector
          """,
          [uuid!(instance_id)]
        )

      updates =
        query_maps(
          """
          SELECT update_record.id, update_record.expected_current_revision_id,
                 update_record.candidate_revision_id,
                 state, hook_run_id, hook_exit_code, hook_exit_signal,
                 diagnostics, final_decision, created_at, updated_at
                 ,(
                   SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'sequence', event.sequence,
                     'event_type', event.event_type,
                     'payload', CASE
                       WHEN event.event_type = 'vm.log'
                       THEN jsonb_build_object(
                         'stream', event.payload -> 'stream',
                         'message', left(event.payload ->> 'message', 4096)
                       )
                       ELSE event.payload
                     END
                   ) ORDER BY event.sequence), '[]'::jsonb)
                   FROM run_events event
                   WHERE event.run_id = update_record.hook_run_id
                 ) AS hook_events
          FROM agent_updates update_record
          WHERE update_record.instance_id = $1
          ORDER BY update_record.created_at DESC, update_record.id
          """,
          [uuid!(instance_id)]
        )

      repositories =
        query_maps(
          """
          SELECT id, name, default_branch
          FROM repositories
          WHERE project_id = $1
          ORDER BY name
          """,
          [uuid!(instance["project_id"])]
        )

      imports =
        query_maps(
          """
          SELECT secret_import.id, secret_import.alias,
                 secret_import.target_kind, secret_import.target_id,
                 secret_import.status, secret.name AS secret_name,
                 secret.status AS secret_status,
                 secret_grant.delivery_modes, secret_grant.phases,
                 secret_grant.destinations, secret_grant.expires_at
          FROM secret_imports secret_import
          JOIN secret_grants secret_grant ON secret_grant.id = secret_import.grant_id
          JOIN secrets secret ON secret.id = secret_import.secret_id
          WHERE secret_import.status = 'active'
            AND secret_grant.status = 'active'
            AND secret.status = 'active'
            AND (
              (
                secret_import.target_kind = 'project'
                AND secret_import.target_id = $1
              )
              OR (
                secret_import.target_kind = 'repository'
                AND secret_import.target_id IN (
                  SELECT repository_id
                  FROM agent_attachments
                  WHERE instance_id = $2
                    AND enabled
                    AND removed_at IS NULL
                )
              )
            )
          ORDER BY secret_import.alias, secret_import.id
          """,
          [uuid!(instance["project_id"]), uuid!(instance_id)]
        )

      candidates =
        query_maps(
          """
          SELECT candidate.id, candidate.display_name,
                 candidate.parameter_schema, candidate.secret_slot_schema,
                 candidate.runtime_contract, candidate.requires_state,
                 candidate.update_hook,
                 release.id AS release_id, release.version AS release_version
          FROM agent_instance_revisions active_revision
          JOIN release_agents active_agent
            ON active_agent.id = active_revision.release_agent_id
          JOIN release_agents candidate
            ON candidate.family_id = active_agent.family_id
          JOIN releases release ON release.id = candidate.release_id
          WHERE active_revision.id = $1
            AND release.state = 'published'
            AND candidate.id <> active_agent.id
            AND check_permission(
              'user', hephaestus_actor_id(), 'can_use',
              'release_agent', candidate.id::text
            ) = 1
          ORDER BY release.created_at DESC, candidate.id
          """,
          [uuid!(instance["active_revision_id"])]
        )

      recent_runs =
        query_maps(
          """
          SELECT id, state, outcome, run_kind, instance_revision_id,
                 release_id, attachment_id, created_at, updated_at
          FROM runs
          WHERE instance_id = $1
          ORDER BY created_at DESC LIMIT 20
          """,
          [uuid!(instance_id)]
        )

      instance
      |> Map.put("revisions", revisions)
      |> Map.put("attachments", attachments)
      |> Map.put("updates", updates)
      |> Map.put("repositories", repositories)
      |> Map.put("secret_imports", imports)
      |> Map.put("update_candidates", candidates)
      |> Map.put("recent_runs", recent_runs)
    end)
  end

  def authorize_run(%Identity{} = identity, run_id) do
    with_actor(identity, fn ->
      Ecto.Adapters.SQL.query!(
        Repo,
        "SELECT check_permission('user', $1, 'can_read', 'run', $2) = 1",
        [identity.user_id, run_id]
      ).rows == [[true]]
    end)
  end

  def get_run(%Identity{} = identity, run_id) do
    with_actor(identity, fn ->
      run =
        query_one(
          """
          SELECT run.id, run.state, run.outcome, run.exit_code, run.exit_signal,
                 run.failure, run.created_at, run.updated_at, run.state_version,
                 instance.id AS agent_id, instance.name AS agent_name,
                 instance_project.id AS instance_project_id,
                 instance_project.name AS instance_project_name,
                 run.instance_revision_id, run.release_id,
                 release.version AS release_version,
                 release.repository_id AS source_repository_id,
                 repository.id AS repository_id, repository.name AS repository_name,
                 project.id AS project_id, project.name AS project_name,
                 organization.id AS organization_id,
                 organization.name AS organization_name,
                 request.commit_sha AS input_commit, request.git_ref,
                 request.attempt,
                 result.id AS result_id, result.result_commit, result.result_ref,
                 result.result_tree, result.message AS result_message,
                 result.artifact_manifest_hash,
                 proposal.id AS proposal_id, proposal.state AS proposal_state,
                 proposal.target_ref, proposal.version AS proposal_version
          FROM runs run
          JOIN agent_instances instance ON instance.id = run.instance_id
          JOIN projects instance_project ON instance_project.id = instance.project_id
          JOIN releases release ON release.id = run.release_id
          JOIN run_requests request ON request.run_id = run.id
          JOIN repositories repository ON repository.id = request.repository_id
          JOIN projects project ON project.id = repository.project_id
          JOIN organizations organization ON organization.id = project.organization_id
          LEFT JOIN run_results result ON result.run_id = run.id
          LEFT JOIN review_proposals proposal ON proposal.run_id = run.id
          WHERE run.id = $1
          """,
          [uuid!(run_id)]
        )

      case run do
        nil ->
          Repo.rollback(:forbidden)

        run ->
          events =
            query_maps(
              """
              SELECT sequence, event_type, payload, occurred_at
              FROM run_events WHERE run_id = $1 ORDER BY sequence
              """,
              [uuid!(run_id)]
            )

          artifacts =
            if run["result_id"] do
              query_maps(
                """
                SELECT id, kind, path, media_type, size_bytes, sha256,
                       storage_key, provenance
                FROM result_artifacts
                WHERE result_id = $1
                ORDER BY
                  CASE kind
                    WHEN 'patch' THEN 0 WHEN 'manifest' THEN 1
                    WHEN 'logs' THEN 2 WHEN 'exit' THEN 3 ELSE 4
                  END,
                  path
                """,
                [uuid!(run["result_id"])]
              )
            else
              []
            end

          logs = Enum.filter(events, &(&1["event_type"] == "vm.log"))

          runtime_metrics =
            events
            |> Enum.filter(&(&1["event_type"] == "vm.metric"))
            |> Enum.reduce(%{}, fn event, latest ->
              Map.put(latest, event["payload"]["name"], event["payload"])
            end)
            |> Map.values()
            |> Enum.sort_by(& &1["name"])

          run
          |> Map.put("events", events)
          |> Map.put("artifacts", artifacts)
          |> Map.put("runtime_metrics", runtime_metrics)
          |> Map.put("metrics", %{
            "event_count" => length(events),
            "log_count" => length(logs),
            "elapsed_ms" => elapsed_ms(run)
          })
      end
    end)
  end

  def create_control(%Identity{} = identity, attributes) do
    request_id = Ecto.UUID.generate()

    with_actor(identity, request_id, fn ->
      control_id = Ecto.UUID.generate()

      Ecto.Adapters.SQL.query!(
        Repo,
        """
        INSERT INTO control_requests
          (id, kind, actor_id, request_id, repository_id,
           run_id, proposal_id, reason)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        """,
        [
          uuid!(control_id),
          Map.fetch!(attributes, "kind"),
          uuid!(identity.user_id),
          uuid!(request_id),
          uuid!(Map.fetch!(attributes, "repository_id")),
          nullable_uuid(attributes["run_id"]),
          nullable_uuid(attributes["proposal_id"]),
          String.slice(attributes["reason"] || "", 0, 4096)
        ]
      )

      control_id
    end)
  end

  def with_actor(%Identity{} = identity, operation) when is_function(operation, 0) do
    with_actor(identity, Ecto.UUID.generate(), operation)
  end

  def with_actor(%Identity{} = identity, request_id, operation)
      when is_function(operation, 0) do
    Repo.transaction(fn ->
      Ecto.Adapters.SQL.query!(
        Repo,
        """
        SELECT set_config('hephaestus.actor_id', $1, true),
               set_config('hephaestus.subject_type', 'user', true),
               set_config('hephaestus.request_id', $2, true)
        """,
        [identity.user_id, request_id]
      )

      operation.()
    end)
  end

  defp query_one(statement, parameters) do
    case query_maps(statement, parameters) do
      [row] -> row
      [] -> nil
    end
  end

  defp query_maps(statement, parameters \\ []) do
    result = Ecto.Adapters.SQL.query!(Repo, statement, parameters)

    Enum.map(result.rows, fn row ->
      result.columns
      |> Enum.zip(row)
      |> Map.new(fn {column, value} -> {column, normalize_value(column, value)} end)
    end)
  end

  defp normalize_value(column, <<_::128>> = value)
       when column == "id" or binary_part(column, byte_size(column) - 3, 3) == "_id",
       do: Ecto.UUID.load!(value)

  defp normalize_value(_column, value), do: value

  defp elapsed_ms(%{"created_at" => created_at, "updated_at" => updated_at}) do
    DateTime.diff(updated_at, created_at, :millisecond)
  end

  defp nullable_uuid(nil), do: nil
  defp nullable_uuid(value), do: uuid!(value)

  defp uuid!(value) do
    case Ecto.UUID.dump(value) do
      {:ok, binary} -> binary
      :error -> raise ArgumentError, "invalid opaque identifier"
    end
  end
end
