defmodule HephaestusWebWeb.BuilderCatalogState do
  @moduledoc "State and effects for the platform-owned builder image catalog."

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
            data: %{builder_images: []},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def statuses, do: @statuses
  def new(_params), do: %__MODULE__{}

  def reduce(state, :load) do
    generation = state.stream_generation + 1
    {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}
  end

  def reduce(state, {:loaded, generation, builder_images})
      when generation == state.stream_generation,
      do: {%{state | status: :ready, data: %{builder_images: builder_images}, error: nil}, []}

  def reduce(state, {:loaded, _generation, _builder_images}), do: {state, []}

  def reduce(state, {:failed, _reason}),
    do: {%{state | status: :error, error: "Builder catalog is unavailable."}, []}

  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def present(state) do
    %{
      state: presentation_state(state),
      builder_images: state.data.builder_images,
      item_count: length(state.data.builder_images),
      error: state.error
    }
  end

  def execute(state, {:load, identity}) do
    generation = state.stream_generation

    case Client.list_builder_images(identity) do
      {:ok, builder_images} -> {:loaded, generation, builder_images}
      {:error, reason} -> {:failed, reason}
    end
  end

  defp presentation_state(%{status: status}) when status in [:ready], do: :ready
  defp presentation_state(%{status: :error}), do: :error

  defp presentation_state(%{status: status}) when status in [:stale, :reconnecting],
    do: :reconnecting

  defp presentation_state(_state), do: :loading
end
