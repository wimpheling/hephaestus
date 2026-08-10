defmodule HephaestusWebWeb.RepositoryBuildsState do
  @moduledoc "C2 state/effects contract for repository build browsing and requests."

  alias HephaestusWebWeb.RepositoryRouteModel
  alias HephaestusWeb.RPC.Client

  @stream_mode :none
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
  def stream_mode, do: @stream_mode
  def new(repository_id), do: RepositoryRouteModel.new(__MODULE__, repository_id)
  def begin_watch(state), do: RepositoryRouteModel.begin_watch(state)

  def watch(identity, state, owner, generation),
    do: RepositoryRouteModel.watch(identity, state, owner, generation)

  def reduce(state, event), do: RepositoryRouteModel.reduce(state, event, :builds)

  def execute(state, {:load, identity, generation}),
    do: RepositoryRouteModel.execute(state, identity, generation, :builds)

  def execute({:request_build, repository_id, attributes}, identity) do
    result =
      Client.request_build(
        identity,
        repository_id,
        Map.fetch!(attributes, "source_commit"),
        Map.fetch!(attributes, "build_definition_hash"),
        Map.fetch!(attributes, "configuration_hash")
      )

    {:request_build_result, result}
  end

  def execute(effect, identity), do: RepositoryRouteModel.execute(effect, identity)
  def present(state), do: RepositoryRouteModel.present(state, :builds)
end
