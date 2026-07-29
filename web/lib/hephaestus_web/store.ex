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
                     run.created_at, run.updated_at, agent.name AS agent_name,
                     request.commit_sha, request.git_ref, request.attempt,
                     proposal.id AS proposal_id, proposal.state AS proposal_state
              FROM run_requests request
              JOIN runs run ON run.id = request.run_id
              JOIN agents agent ON agent.id = run.agent_id
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
                 agent.id AS agent_id, agent.name AS agent_name,
                 repository.id AS repository_id, repository.name AS repository_name,
                 project.id AS project_id, project.name AS project_name,
                 organization.id AS organization_id,
                 organization.name AS organization_name,
                 request.commit_sha AS input_commit, request.git_ref,
                 request.config_hash, request.attempt,
                 result.id AS result_id, result.result_commit, result.result_ref,
                 result.result_tree, result.message AS result_message,
                 result.artifact_manifest_hash,
                 proposal.id AS proposal_id, proposal.state AS proposal_state,
                 proposal.target_ref, proposal.version AS proposal_version
          FROM runs run
          JOIN agents agent ON agent.id = run.agent_id
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
