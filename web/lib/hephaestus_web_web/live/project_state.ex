defmodule HephaestusWebWeb.ProjectState do
  @moduledoc "State and effects for the project repositories route."

  alias HephaestusWeb.RPC.{Client, Error, ProductEvents}
  alias HephaestusWebWeb.ProductEventReducer

  @stream_mode :page_scoped
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
            data: %{
              project_id: nil,
              project: nil,
              repositories: [],
              recipe_inspection: %{
                status: :idle,
                deployment_id: "",
                deployment: nil,
                error: nil,
                generation: 0
              }
            },
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}) do
    state = %__MODULE__{}
    %{state | data: %{state.data | project_id: project_id}}
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:project, state.data.project_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      fn response ->
        send(owner, {:page_watch, generation, response})

        case response.item do
          {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
          _item -> :cont
        end
      end
    )
  end

  def reduce(state, {:watch, response}) do
    {next, effects} =
      ProductEventReducer.reduce(state, response, [:project_changed, :repository_changed])

    if next.status == :access_revoked do
      {revoke_inspection(next, state.data.recipe_inspection), effects}
    else
      {next, effects}
    end
  end

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:loaded, generation, project, repositories})
      when generation == state.stream_generation do
    data = %{state.data | project: project, repositories: repositories}
    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  # A snapshot already in flight may finish after a transient watch reconnect.
  # Keep the stale-generation guard for ordinary loads, but do not discard that
  # initial snapshot while the page is recovering its stream.
  def reduce(%{status: :reconnecting} = state, {:loaded, _generation, project, repositories}) do
    data = %{state.data | project: project, repositories: repositories}
    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _stale_generation, _project, _repositories}), do: {state, []}

  def reduce(state, {:access_revoked, _reason}) do
    revoked = %{state | status: :access_revoked, error: "Project access was revoked."}
    {revoke_inspection(revoked, state.data.recipe_inspection), [{:navigate, :organizations}]}
  end

  def reduce(state, {:error, message}) when is_binary(message),
    do: {%{state | status: :error, error: message}, []}

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def reduce(state, {:inspect_recipe, deployment_id}) when is_binary(deployment_id) do
    generation = state.data.recipe_inspection.generation + 1

    inspection = %{
      status: :loading,
      deployment_id: deployment_id,
      deployment: nil,
      error: nil,
      generation: generation
    }

    if byte_size(deployment_id) == 36 do
      {put_in(state.data.recipe_inspection, inspection),
       [{:inspect_recipe, generation, deployment_id}]}
    else
      invalid = %{inspection | status: :invalid, error: "Enter the exact deployment UUID."}
      {put_in(state.data.recipe_inspection, invalid), []}
    end
  end

  def reduce(state, {:recipe_inspected, generation, result})
      when state.status != :access_revoked and
             generation == state.data.recipe_inspection.generation do
    inspection = state.data.recipe_inspection

    inspection =
      case result do
        {:ok, deployment} ->
          %{inspection | status: :ready, deployment: deployment, error: nil}

        {:error, %Error{kind: kind}} when kind in [:permission_denied, :unauthenticated] ->
          %{
            inspection
            | status: :denied,
              deployment: nil,
              error: "Your current session cannot inspect this deployment.",
              generation: inspection.generation + 1
          }

        {:error, %Error{kind: :invalid}} ->
          %{
            inspection
            | status: :invalid,
              deployment: nil,
              error: "Enter the exact deployment UUID."
          }

        {:error, %Error{kind: :precondition}} ->
          %{
            inspection
            | status: :error,
              deployment: nil,
              error: "The saved deployment could not be validated."
          }

        {:error, _reason} ->
          %{
            inspection
            | status: :unavailable,
              deployment: nil,
              error:
                "Deployment inspection is unavailable. No absence or completion is confirmed."
          }
      end

    {put_in(state.data.recipe_inspection, inspection), []}
  end

  def reduce(state, {:recipe_inspected, _generation, _result}), do: {state, []}

  def present(state) do
    Map.merge(state.data, %{
      status: presentation_status(state),
      item_count: length(state.data.repositories),
      error: state.error,
      recipe_inspection: state.data.recipe_inspection
    })
  end

  def execute(state, {:load, identity, generation}) do
    project_id = state.data.project_id

    with {:ok, project} <- Client.get_project(identity, project_id),
         {:ok, repositories} <- Client.list_project_repositories(identity, project_id) do
      {:loaded, generation, project, repositories}
    else
      {:error, %Error{kind: kind} = reason} when kind in [:not_found, :permission_denied] ->
        {:access_revoked, reason}

      {:error, %Error{} = reason} ->
        {:error, Error.present(reason)}

      {:error, _reason} ->
        {:error, "The project data is temporarily unavailable."}
    end
  end

  def execute_recipe(state, {:inspect_recipe, generation, deployment_id}, identity, options \\ []) do
    {:recipe_inspected, generation,
     Client.get_recipe_deployment(identity, state.data.project_id, deployment_id, options)}
  end

  defp revoke_inspection(state, inspection) do
    denied = %{
      inspection
      | status: :denied,
        deployment: nil,
        error: "Project access was revoked.",
        generation: inspection.generation + 1
    }

    %{state | data: Map.put(state.data, :recipe_inspection, denied)}
  end

  defp presentation_status(%{status: :ready}), do: :ready
  defp presentation_status(%{status: :reconnecting}), do: :reconnecting

  defp presentation_status(%{status: status}) when status in [:initial, :loading, :stale],
    do: :loading

  defp presentation_status(_state), do: :error
end
