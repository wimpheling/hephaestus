defmodule HephaestusWebWeb.BuildState do
  @moduledoc "State, reducer, presentation, and typed effects for a build detail."

  alias HephaestusWeb.RPC.{Client, Error, ProductEvents}
  alias HephaestusWebWeb.ProductEventReducer

  @statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]
  defstruct status: :initial,
            data: %{},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def statuses, do: @statuses
  def stream_mode, do: :page_scoped
  def new(build_id), do: new(nil, build_id)

  def new(repository_id, build_id),
    do: %__MODULE__{data: %{repository_id: repository_id, build_id: build_id}}

  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:repository, state.data.repository_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  def reduce(state, :load) do
    generation = state.stream_generation + 1
    effect = {:load, generation, state.data.repository_id, state.data.build_id}
    {%{state | status: :loading, error: nil, stream_generation: generation}, [effect]}
  end

  def reduce(state, :disconnected), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :connected), do: reduce(state, :load)

  def reduce(state, {:watch, response}),
    do: ProductEventReducer.reduce(state, response, [:build_changed])

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(
        %{data: %{build_id: build_id, build: %{"state" => state}}, stream_generation: generation} =
          page_state,
        :retry_attempt
      )
      when state in ["failed", "cancelled"],
      do: action_requested(page_state, generation, :retry, build_id)

  def reduce(state, :retry_attempt),
    do: unavailable_action(state, "BuildService.RetryBuild is not allowed for this state")

  def reduce(
        %{
          data: %{build_id: build_id, build: %{"state" => "succeeded"}},
          stream_generation: generation
        } = page_state,
        :rebuild_for_verification
      ),
      do: action_requested(page_state, generation, :verification, build_id)

  def reduce(state, :rebuild_for_verification),
    do:
      unavailable_action(
        state,
        "BuildService.RebuildForVerification is not allowed for this state"
      )

  def reduce(state, {:build_another_commit, _commit}),
    do: unavailable_action(state, "BuildService.RequestBuild for another commit")

  def reduce(
        %{stream_generation: generation} = state,
        {:action_result, generation, kind, {:ok, _result}}
      ) do
    message = if kind == :retry, do: "Build retry queued.", else: "Verification rebuild queued."
    {%{state | status: :ready, error: nil}, [{:flash, :info, message}, :snapshot]}
  end

  def reduce(
        %{stream_generation: generation} = state,
        {:action_result, generation, _kind, {:error, reason}}
      ) do
    {%{state | status: :error, error: Error.present(reason)},
     [{:flash, :error, "Build action was rejected."}]}
  end

  def reduce(%{stream_generation: generation} = state, {:loaded, generation, {:ok, data}}) do
    data = normalize_loaded(data)

    state
    |> Map.put(:status, :ready)
    |> Map.put(:error, nil)
    |> Map.update!(:data, &Map.merge(&1, data))
    |> ProductEventReducer.snapshot_complete()
  end

  def reduce(%{stream_generation: generation} = state, {:loaded, generation, {:error, _reason}}) do
    {%{state | status: :access_revoked, error: "Build not found or access was revoked."},
     [{:flash, :error, "Build not found or access was revoked."}, {:navigate, "/organizations"}]}
  end

  def reduce(state, {:loaded, _stale_generation, _result}), do: {state, []}

  def reduce(state, {:effect_failed, _reason}),
    do: {%{state | status: :error, error: "Build data is temporarily unavailable."}, []}

  def present(state) do
    build = state.data[:build]
    repository = state.data[:repository]

    %{
      state: page_state(state.status),
      build: build,
      repository: repository,
      logs: bounded_logs(build),
      metrics: (build && build["metrics"]) || [],
      timeline: (build && build["timeline"]) || [],
      declared_artifacts: (build && build["declared_artifacts"]) || [],
      produced_artifacts: (build && build["produced_artifacts"]) || [],
      artifact_manifest: (build && build["artifact_manifest"]) || [],
      verifications: (build && build["verifications"]) || [],
      retry_event: retry_event(build),
      verification_rebuild_event: verification_event(build),
      another_commit_event: nil,
      destinations: destinations(repository, state.data.repository_id, build),
      error: state.error
    }
  end

  def execute({:load, generation, repository_id, build_id}, identity) do
    {:loaded, generation, load(identity, repository_id, build_id)}
  end

  def execute({:action, generation, :retry, build_id}, identity),
    do: {:action_result, generation, :retry, Client.retry_build(identity, build_id)}

  def execute({:action, generation, :verification, build_id}, identity),
    do:
      {:action_result, generation, :verification,
       Client.rebuild_for_verification(identity, build_id)}

  def execute(state, {:load, identity, generation}) do
    execute(
      {:load, generation, state.data.repository_id, state.data.build_id},
      identity
    )
  end

  defp load(identity, repository_id, build_id) do
    with {:ok, repository} <- Client.get_repository(identity, repository_id),
         {:ok, build} <- Client.get_build(identity, build_id) do
      {:ok, %{repository: repository, build: build}}
    end
  end

  defp normalize_loaded(%{"build" => _build} = data), do: data
  defp normalize_loaded(%{build: _build} = data), do: data
  defp normalize_loaded(build), do: %{build: build, repository: nil}

  defp destinations(nil, nil, _build), do: %{}

  defp destinations(nil, repository_id, build) do
    %{
      repository: "/repositories/#{repository_id}/builds",
      release: release_destination(repository_id, build)
    }
  end

  defp destinations(repository, _repository_id, build) do
    %{
      organization_index: "/organizations",
      organization: "/organizations/#{repository["organization_id"]}",
      project: "/projects/#{repository["project_id"]}",
      repository: "/repositories/#{repository["id"]}/builds",
      release: release_destination(repository["id"], build)
    }
  end

  defp release_destination(repository_id, build) do
    case build && build["release_id"] do
      nil -> nil
      release_id -> "/repositories/#{repository_id}/releases/#{release_id}"
    end
  end

  defp bounded_logs(nil), do: []

  defp bounded_logs(build),
    do: build["logs"] |> Kernel.||([]) |> Enum.take(200) |> Enum.map(&String.slice(&1, 0, 16_384))

  defp page_state(status) when status in [:initial, :loading, :submitting], do: :loading

  defp page_state(:ready), do: :ready
  defp page_state(status) when status in [:stale, :reconnecting], do: :reconnecting
  defp page_state(_status), do: :error

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end

  defp retry_event(%{"state" => state}) when state in ["failed", "cancelled"], do: "retry-build"
  defp retry_event(_build), do: nil

  defp verification_event(%{"state" => "succeeded"}), do: "verification-rebuild"
  defp verification_event(_build), do: nil

  defp action_requested(state, generation, kind, build_id),
    do: {%{state | status: :submitting, error: nil}, [{:action, generation, kind, build_id}]}

  defp unavailable_action(state, operation) do
    message = "#{operation} is not present in the generated BuildService client."
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end
end
