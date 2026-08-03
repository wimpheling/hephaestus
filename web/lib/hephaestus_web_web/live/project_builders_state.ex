defmodule HephaestusWebWeb.ProjectBuildersState do
  @moduledoc "State and effects for a project's owned OCI builders."

  alias HephaestusWeb.RPC.Client

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
            data: %{project_id: nil, builders: []},
            form: %{},
            error: nil,
            stream_generation: 0

  def statuses, do: @statuses

  def new(%{project_id: project_id}),
    do: %__MODULE__{data: %{project_id: project_id, builders: []}}

  def reduce(state, :load) do
    generation = state.stream_generation + 1
    {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}
  end

  def reduce(state, {:loaded, generation, builders}) when generation == state.stream_generation,
    do: {%{state | status: :ready, data: %{state.data | builders: builders}, error: nil}, []}

  def reduce(state, {:loaded, _generation, _builders}), do: {state, []}

  def reduce(state, {:failed, _reason}),
    do: {%{state | status: :error, error: "Project builders are unavailable."}, []}

  def reduce(state, :stale), do: %{state | status: :stale}
  def reduce(state, :reconnecting), do: %{state | status: :reconnecting}
  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:created, builder}) do
    {%{state | status: :ready, data: %{state.data | builders: [builder | state.data.builders]}}, []}
  end

  def reduce(state, {:prepared, builder}) do
    builders = Enum.map(state.data.builders, fn item -> if item["id"] == builder["id"], do: builder, else: item end)
    {%{state | status: :ready, data: %{state.data | builders: builders}}, []}
  end

  def present(state) do
    %{
      state: presentation_state(state),
      project_id: state.data.project_id,
      builders: state.data.builders,
      item_count: length(state.data.builders),
      error: state.error
    }
  end

  def execute(state, {:load, identity}) do
    generation = state.stream_generation

    case Client.list_project_builders(identity, state.data.project_id) do
      {:ok, builders} -> {:loaded, generation, builders}
      {:error, reason} -> {:failed, reason}
    end
  end

  def execute(state, {:create, identity, attributes}) do
    case Client.create_project_builder(
           identity,
           state.data.project_id,
           attributes["source_repository_id"],
           attributes
         ) do
      {:ok, builder} -> {:created, builder}
      {:error, reason} -> {:failed, reason}
    end
  end

  def execute(state, {:prepare, identity, builder_id}) do
    case Client.request_project_builder_preparation(identity, state.data.project_id, builder_id) do
      {:ok, builder} -> {:prepared, builder}
      {:error, reason} -> {:failed, reason}
    end
  end

  defp presentation_state(%{status: :ready}), do: :ready

  defp presentation_state(%{status: status}) when status in [:stale, :reconnecting],
    do: :reconnecting

  defp presentation_state(%{status: :error}), do: :error
  defp presentation_state(_state), do: :loading
end
