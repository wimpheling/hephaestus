defmodule HephaestusWebWeb.OrganizationState do
  @moduledoc "State and backend effects for the organization index."

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
            data: %{organizations: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(_params), do: %__MODULE__{}
  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(_state), do: :identity

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
      :identity_profile_changed,
      :identity_organizations_changed
    ])
  end

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, :load), do: {%{state | status: :loading, error: nil}, [:load]}

  def reduce(state, {:loaded, organizations}),
    do: reduce(state, {:loaded, state.stream_generation, organizations})

  def reduce(state, {:loaded, generation, organizations})
      when generation == state.stream_generation do
    state = %{state | data: %{state.data | organizations: organizations}}
    ProductEventReducer.snapshot_complete(state)
  end

  def reduce(state, {:loaded, _generation, _organizations}), do: {state, []}

  def reduce(state, {:failed, reason}) do
    message = present_error(reason)
    {%{state | status: :error, data: %{organizations: []}, error: message}, []}
  end

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def present(state) do
    organizations = state.data.organizations

    %{
      status: presentation_status(state.status, organizations),
      organizations: organizations,
      organization_count: length(organizations),
      error: state.error
    }
  end

  def execute(_state, {:load, identity, generation}) do
    case Client.list_organizations(identity) do
      {:ok, organizations} -> {:loaded, generation, organizations}
      {:error, reason} -> {:failed, reason}
    end
  end

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end

  defp presentation_status(:ready, []), do: :empty
  defp presentation_status(:ready, _organizations), do: :ready
  defp presentation_status(:reconnecting, _organizations), do: :reconnecting

  defp presentation_status(status, _organizations) when status in [:initial, :loading],
    do: :loading

  defp presentation_status(_status, _organizations), do: :error

  defp present_error(%Error{} = error), do: Error.present(error)
  defp present_error(_reason), do: "Organizations are temporarily unavailable."
end
