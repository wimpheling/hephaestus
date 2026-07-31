defmodule HephaestusWebWeb.PageStreamProbeState do
  @moduledoc false

  defstruct status: :ready,
            data: %{},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def begin_watch(state),
    do: %{state | stream_generation: state.stream_generation + 1}

  def watch(_identity, state, _owner, generation) do
    send(state.data.test_owner, {
      :probe_watch_started,
      self(),
      generation,
      state.cursor && state.cursor.committed
    })

    receive do
      :finish -> :ok
    end
  end
end
