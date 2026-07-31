defmodule HephaestusWebWeb.OrganizationWorkspaceState do
  @moduledoc "State and backend effects for the organization projects route."

  alias HephaestusWeb.RPC.{Client, ProductEvents}
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
            data: %{organization_id: nil, organization: nil, projects: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{organization_id: organization_id}) do
    %__MODULE__{data: %{organization_id: organization_id, organization: nil, projects: []}}
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:organization, state.data.organization_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  def reduce(state, {:watch, response}) do
    ProductEventReducer.reduce(state, response, [
      :organization_changed,
      :project_changed,
      :repository_changed
    ])
  end

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, :load), do: reduce(state, {:load, state.stream_generation + 1})

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:loaded, organization, projects}),
    do: reduce(state, {:loaded, state.stream_generation, organization, projects})

  def reduce(state, {:loaded, generation, organization, projects})
      when generation == state.stream_generation do
    data = %{state.data | organization: organization, projects: projects}
    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _generation, _organization, _projects}), do: {state, []}

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Organization access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def present(state) do
    %{
      status: presentation_status(state),
      organization: state.data.organization,
      projects: state.data.projects,
      error: state.error
    }
  end

  def execute(state, {:load, identity, generation}) do
    organization_id = state.data.organization_id

    with {:ok, organization} <- Client.get_organization(identity, organization_id),
         {:ok, projects} <- Client.list_projects(identity, organization_id) do
      {:loaded, generation, organization, projects}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  defp presentation_status(%{status: :ready}), do: :ready
  defp presentation_status(%{status: :reconnecting}), do: :reconnecting
  defp presentation_status(%{status: status}) when status in [:initial, :loading], do: :loading
  defp presentation_status(_state), do: :error

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
