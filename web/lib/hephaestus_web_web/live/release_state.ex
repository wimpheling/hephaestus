defmodule HephaestusWebWeb.ReleaseState do
  @moduledoc "State, reducer, presentation, and backend effects for a release."

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
            data: %{},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  @type t :: %__MODULE__{
          status: atom(),
          data: map(),
          form: map(),
          error: String.t() | nil,
          cursor: term(),
          stream_generation: non_neg_integer()
        }

  @doc "Returns every state accepted by the lifecycle reducer."
  @spec statuses() :: [atom()]
  def statuses, do: @statuses

  @doc "Declares that this page owns a repository-scoped product-event watch."
  @spec stream_mode() :: :page_scoped
  def stream_mode, do: @stream_mode

  @doc "Creates inert initial state; effects begin only through the reducer."
  @spec new(String.t()) :: t()
  def new(release_id), do: %__MODULE__{data: %{release_id: release_id}}

  @doc "Purely reduces lifecycle and effect-result messages."
  @spec reduce(t(), term()) :: {t(), [term()]}
  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:repository, state.data.release["repository_id"]}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  def reduce(%__MODULE__{} = state, :load) do
    generation = state.stream_generation + 1

    {%{state | status: :loading, error: nil, stream_generation: generation},
     [{:load, generation, state.data.release_id}]}
  end

  def reduce(%__MODULE__{} = state, :disconnected),
    do: {%{state | status: :reconnecting}, []}

  def reduce(%__MODULE__{} = state, :connected), do: reduce(state, :load)

  def reduce(%__MODULE__{} = state, {:watch, response}) do
    ProductEventReducer.reduce(state, response, [
      :repository_changed,
      :build_changed,
      :release_changed,
      :artifact_changed
    ])
  end

  def reduce(%__MODULE__{} = state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :loaded,
        generation,
        {:ok, release}
      }) do
    data =
      Map.merge(state.data, %{
        release_id: state.data.release_id,
        release: release,
        artifacts: release["artifacts"],
        agents: release["agents"]
      })

    %{state | data: data, error: nil} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :loaded,
        generation,
        {:error, _reason}
      }) do
    {%{state | status: :access_revoked, error: "That release is not visible."},
     [{:navigate, "/organizations"}]}
  end

  def reduce(%__MODULE__{} = state, {:loaded, _stale_generation, _result}), do: {state, []}

  def reduce(%__MODULE__{} = state, {:effect_failed, _reason}) do
    {%{state | status: :error, error: "The release could not be loaded."}, []}
  end

  @doc "Runs a reducer-issued effect outside the LiveView process."
  @spec execute(term(), map()) :: term()
  def execute({:load, generation, release_id}, identity) do
    {:loaded, generation, Client.get_release(identity, release_id)}
  end

  def execute(state, {:load, identity, generation}) do
    {:loaded, generation, Client.get_release(identity, state.data.release_id)}
  end

  @doc "Builds the pure page presentation model."
  @spec present(t()) :: map()
  def present(%__MODULE__{} = state) do
    release = state.data[:release]

    %{
      state: page_state(state.status),
      release: release,
      artifacts: state.data[:artifacts] || [],
      agents: state.data[:agents] || [],
      error: state.error,
      destinations: release_destinations(release)
    }
  end

  defp page_state(status) when status in [:initial, :loading, :submitting], do: :loading
  defp page_state(:ready), do: :ready
  defp page_state(status) when status in [:stale, :reconnecting], do: :reconnecting
  defp page_state(_status), do: :error

  defp release_destinations(nil), do: %{}

  defp release_destinations(release) do
    %{
      organization_index: "/organizations",
      organization: "/organizations/#{release["organization_id"]}",
      project: "/projects/#{release["project_id"]}",
      repository_releases: "/repositories/#{release["repository_id"]}/releases",
      source:
        "/repositories/#{release["repository_id"]}/commits?ref=#{URI.encode_www_form(release["source_ref"])}"
    }
  end

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
