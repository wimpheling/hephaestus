defmodule HephaestusWebWeb.RunState do
  @moduledoc "State, reducer, cursor, presentation, and effects for an exact run."

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
  @control_kinds ["cancel_run", "retry_run", "approve_result", "reject_result"]

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

  @doc "Declares a page-scoped run event stream."
  @spec stream_mode() :: :page_scoped
  def stream_mode, do: @stream_mode

  @doc "Creates inert initial state for one authorized run."
  @spec new(String.t()) :: t()
  def new(run_id), do: %__MODULE__{data: %{run_id: run_id}}

  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:run, state.data.run_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  @doc "Purely reduces lifecycle, stream, control, and effect-result messages."
  @spec reduce(t(), term()) :: {t(), [term()]}
  def reduce(%__MODULE__{} = state, :load), do: begin_load(state, :loading)

  def reduce(%__MODULE__{} = state, {:watch, response}) do
    ProductEventReducer.reduce(state, response, [
      :run_changed,
      :review_changed,
      :artifact_changed
    ])
  end

  def reduce(%__MODULE__{} = state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(%__MODULE__{} = state, :refresh), do: begin_load(state, :stale)
  def reduce(%__MODULE__{} = state, :disconnected), do: {%{state | status: :reconnecting}, []}

  def reduce(%__MODULE__{data: %{run: run}} = state, :connected) when not is_nil(run),
    do: begin_load(state, :stale)

  def reduce(%__MODULE__{} = state, :connected), do: begin_load(state, :loading)

  def reduce(%__MODULE__{data: %{run: run}} = state, {:control, %{"kind" => kind} = params})
      when kind in @control_kinds and not is_nil(run) do
    generation = state.stream_generation
    payload = control_payload(run, kind, params)

    {%{state | status: :submitting, error: nil, stream_generation: generation},
     [{:control, generation, payload}]}
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :loaded,
        generation,
        {:ok, run}
      })
      when is_map(run) do
    state |> snapshot_data(run) |> ProductEventReducer.snapshot_complete()
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :loaded,
        generation,
        _unavailable
      }) do
    message = "Run not found or access was revoked."

    {%{state | status: :access_revoked, error: message},
     [{:flash, :error, message}, {:navigate, "/organizations"}]}
  end

  def reduce(%__MODULE__{} = state, {:loaded, _stale_generation, _result}), do: {state, []}

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :control_completed,
        generation,
        {:ok, receipt, message}
      }) do
    {state, snapshot_effects} = ProductEventReducer.await_receipt(state, receipt)

    {state, [{:flash, :info, message} | snapshot_effects]}
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :control_completed,
        generation,
        {:error, _reason}
      }) do
    message = "Control was denied or could not be completed."
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  def reduce(%__MODULE__{} = state, {:control_completed, _stale_generation, _result}),
    do: {state, []}

  def reduce(%__MODULE__{} = state, {:effect_failed, _reason}) do
    message = "Run updates are temporarily unavailable."
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  def watch_ended(state, {:error, %HephaestusWeb.RPC.Error{kind: kind}})
      when kind in [:not_found, :permission_denied, :unauthenticated] do
    message = "Run not found or access was revoked."

    {%{state | status: :access_revoked, error: message},
     [{:flash, :error, message}, {:navigate, "/organizations"}]}
  end

  def watch_ended(state, _result), do: reduce(state, :watch_ended)

  @doc "Executes a reducer-issued load or control effect outside the LiveView process."
  @spec execute(term(), map()) :: term()
  def execute({:load, generation, run_id}, identity) do
    {:loaded, generation, Client.get_run(identity, run_id)}
  end

  def execute(state, {:load, identity, generation}) do
    execute({:load, generation, state.data.run_id}, identity)
  end

  def execute({:control, generation, payload}, identity) do
    result =
      with {:ok, response} <- Client.create_control(identity, payload),
           {:ok, receipt} <- ProductEventReducer.receipt(response) do
        {:ok, receipt, control_message(payload["kind"])}
      end

    {:control_completed, generation, result}
  end

  @doc "Builds the pure run-page presentation model."
  @spec present(t()) :: map()
  def present(%__MODULE__{} = state) do
    run = state.data[:run]

    %{
      state: page_state(state.status),
      run: run,
      patch: state.data[:patch],
      manifest: state.data[:manifest],
      events: state.data[:events] || [],
      artifacts: state.data[:artifacts] || [],
      error: state.error,
      destinations: destinations(run)
    }
  end

  defp begin_load(state, status) when status in [:loading, :stale] do
    generation = state.stream_generation + 1

    {%{state | status: status, error: nil, stream_generation: generation},
     [{:load, generation, state.data.run_id}]}
  end

  defp snapshot_data(state, run) do
    events = Enum.reverse(run["events"])

    data =
      Map.merge(state.data, %{
        run_id: state.data.run_id,
        run: run,
        patch: run["patch_preview"],
        manifest: run["manifest_preview"],
        events: events,
        artifacts: run["artifacts"]
      })

    %{state | data: data}
  end

  defp control_payload(run, kind, params) do
    %{
      "kind" => kind,
      "repository_id" => run["repository_id"],
      "run_id" => if(kind in ["cancel_run", "retry_run"], do: run["id"]),
      "proposal_id" => if(kind in ["approve_result", "reject_result"], do: run["proposal_id"]),
      "reason" => params["reason"] || "",
      "run_lookup_id" => run["id"]
    }
  end

  defp page_state(status) when status in [:initial, :loading, :submitting], do: :loading
  defp page_state(:ready), do: :ready
  defp page_state(status) when status in [:stale, :reconnecting], do: :reconnecting
  defp page_state(_status), do: :error

  defp destinations(nil), do: %{}

  defp destinations(run) do
    %{
      organization_index: "/organizations",
      organization: "/organizations/#{run["organization_id"]}",
      repository: "/repositories/#{run["repository_id"]}",
      release: "/repositories/#{run["source_repository_id"]}/releases/#{run["release_id"]}",
      agent: "/projects/#{run["instance_project_id"]}/agents/#{run["agent_id"]}"
    }
  end

  defp control_message("approve_result"),
    do: "Approval queued. The host will CAS fast-forward the target."

  defp control_message("reject_result"), do: "Rejection queued."
  defp control_message("retry_run"), do: "Retry queued from the exact accepted input."
  defp control_message("cancel_run"), do: "Cancellation queued."

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
