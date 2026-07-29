defmodule HephaestusWebWeb.RunLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWeb.{ArtifactStore, RunNotifier, Store}

  @refresh_interval 1_000

  @impl true
  def mount(%{"run_id" => run_id}, _session, socket) do
    identity = socket.assigns.current_identity

    with {:ok, true} <- Store.authorize_run(identity, run_id),
         {:ok, run} <- Store.get_run(identity, run_id) do
      if connected?(socket) do
        :ok = RunNotifier.subscribe(run_id)
        Process.send_after(self(), :reauthorize, @refresh_interval)
      end

      {:ok,
       socket
       |> stream_configure(:events, dom_id: &"event-#{&1["sequence"]}")
       |> stream_configure(:artifacts, dom_id: &"artifact-#{&1["id"]}")
       |> assign(:page_title, "Run #{String.slice(run_id, 0, 8)}")
       |> assign(:run_id, run_id)
       |> assign_run(run)}
    else
      _ ->
        {:ok,
         socket
         |> put_flash(:error, "Run not found or access was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def handle_info({:run_wakeup, run_id}, %{assigns: %{run_id: run_id}} = socket) do
    refresh(socket)
  end

  def handle_info(:reauthorize, socket) do
    Process.send_after(self(), :reauthorize, @refresh_interval)
    refresh(socket)
  end

  @impl true
  def handle_event("control", %{"kind" => kind} = params, socket) do
    run = socket.assigns.run

    attributes = %{
      "kind" => kind,
      "repository_id" => run["repository_id"],
      "run_id" => if(kind in ["cancel_run", "retry_run"], do: run["id"]),
      "proposal_id" => if(kind in ["approve_result", "reject_result"], do: run["proposal_id"]),
      "reason" => params["reason"] || ""
    }

    case Store.create_control(socket.assigns.current_identity, attributes) do
      {:ok, _control_id} ->
        {:noreply,
         socket
         |> put_flash(:info, control_message(kind))
         |> refresh_now()}

      {:error, reason} ->
        {:noreply, put_flash(socket, :error, "Control rejected: #{inspect(reason)}")}
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <.breadcrumbs id="run-breadcrumbs">
        <:item navigate={~p"/organizations"}>Organizations</:item>
        <:item navigate={~p"/organizations/#{@run["organization_id"]}"}>
          {@run["organization_name"]}
        </:item>
        <:item navigate={~p"/repositories/#{@run["repository_id"]}"}>
          {@run["repository_name"]}
        </:item>
        <:current>Run {short_sha(@run["id"])}</:current>
      </.breadcrumbs>

      <section class="run-hero">
        <div>
          <div class="run-title-line">
            <span class={["run-state large", state_class(@run)]}><i></i></span>
            <div>
              <p class="eyebrow">Agent run · attempt {@run["attempt"]}</p>
              <h1>{@run["agent_name"]}</h1>
            </div>
          </div>
          <p class="lede">
            Exact input <code>{short_sha(@run["input_commit"])}</code>
            on <code>{@run["git_ref"]}</code>
          </p>
          <div class="provenance-links" id="run-exact-provenance">
            <.link navigate={
              ~p"/repositories/#{@run["source_repository_id"]}/releases/#{@run["release_id"]}"
            }>
              Release {@run["release_version"]}
            </.link>
            <span>·</span>
            <.link navigate={~p"/projects/#{@run["instance_project_id"]}/agents/#{@run["agent_id"]}"}>
              Revision {short_sha(@run["instance_revision_id"])}
            </.link>
            <span>·</span>
            <code>target {short_sha(@run["input_commit"])}</code>
          </div>
        </div>
        <div class="control-bar">
          <button
            :if={active?(@run)}
            phx-click="control"
            phx-value-kind="cancel_run"
            class="button secondary"
            data-testid="cancel-run"
          >Cancel</button>
          <button
            phx-click="control"
            phx-value-kind="retry_run"
            class="button secondary"
            data-testid="retry-run"
          >Retry exact input</button>
        </div>
      </section>

      <div class="metric-grid">
        <.metric label="State" value={human_state(@run["outcome"] || @run["state"])} />
        <.metric label="Duration" value={"#{@run["metrics"]["elapsed_ms"]} ms"} />
        <.metric label="Events" value={@run["metrics"]["event_count"]} />
        <.metric label="Log frames" value={@run["metrics"]["log_count"]} />
      </div>

      <section
        :if={@run["runtime_metrics"] != []}
        id="runtime-metrics"
        class="panel runtime-metrics"
        data-testid="runtime-metrics"
      >
        <div class="panel-heading">
          <div>
            <p class="eyebrow">Guest telemetry</p><h2>Latest samples</h2>
          </div>
          <.tag tone="success" dot>persisted</.tag>
        </div>
        <div class="metric-grid">
          <.metric
            :for={metric <- @run["runtime_metrics"]}
            label={metric["name"]}
            value={metric_value(metric)}
          />
        </div>
      </section>

      <section :if={@run["proposal_id"]} class="proposal-card" data-testid="review-proposal">
        <div class="proposal-heading">
          <div>
            <p class="eyebrow">Controlled result proposal</p>
            <h2>{@run["target_ref"]}</h2>
          </div>
          <.tag tone={proposal_tone(@run["proposal_state"])}>{@run["proposal_state"]}</.tag>
        </div>
        <div class="commit-flow">
          <div><small>Input</small><code>{short_sha(@run["input_commit"])}</code></div>
          <span>→</span>
          <div><small>Result</small><code>{short_sha(@run["result_commit"])}</code></div>
          <span>→</span>
          <div><small>Target</small><code>{@run["target_ref"]}</code></div>
        </div>
        <p class="result-message">“{@run["result_message"]}”</p>
        <div :if={@run["proposal_state"] in ["open", "approval_requested"]} class="review-actions">
          <button
            phx-click="control"
            phx-value-kind="reject_result"
            class="button danger"
            data-testid="reject-result"
          >Reject</button>
          <button
            phx-click="control"
            phx-value-kind="approve_result"
            class="button primary"
            data-testid="approve-result"
          >Approve fast-forward</button>
        </div>
      </section>

      <div class="review-grid">
        <section class="panel diff-panel">
          <div class="panel-heading">
            <div>
              <p class="eyebrow">Workspace result</p><h2>Patch</h2>
            </div>
            <.tag>{artifact_size(@run, "patch")}</.tag>
          </div>
          <pre id="result-diff" data-testid="result-diff"><code>{@patch || "No result patch has been published yet."}</code></pre>
        </section>

        <section class="panel timeline-panel">
          <div class="panel-heading">
            <div>
              <p class="eyebrow">Persisted lifecycle</p><h2>Timeline</h2>
            </div>
            <.tag tone="success" dot>live</.tag>
          </div>
          <ol
            id="run-timeline"
            class="timeline"
            data-testid="run-timeline"
            phx-update="stream"
          >
            <li :for={{dom_id, event} <- @streams.events} id={dom_id}>
              <span class="timeline-dot"></span>
              <div>
                <strong>{event_label(event["event_type"])}</strong>
                <small>#{event["sequence"]} · {Calendar.strftime(event["occurred_at"], "%H:%M:%S")}</small>
                <code :if={event["event_type"] == "vm.log"}>{log_line(event["payload"])}</code>
              </div>
            </li>
          </ol>
        </section>
      </div>

      <section class="panel artifact-panel">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">Durable provenance</p><h2>Artifacts</h2>
          </div>
          <code>{short_sha(@run["artifact_manifest_hash"])}</code>
        </div>
        <div id="artifacts" class="artifact-list" phx-update="stream">
          <div
            :for={{dom_id, artifact} <- @streams.artifacts}
            id={dom_id}
            class="artifact-row"
          >
            <.tag tone="accent">{artifact["kind"]}</.tag>
            <span>{artifact["path"] || "run output"}</span>
            <code>{format_bytes(artifact["size_bytes"])}</code>
            <code>{short_sha(artifact["sha256"])}</code>
          </div>
        </div>
        <details :if={@manifest}>
          <summary>Inspect manifest JSON</summary>
          <pre><code>{@manifest}</code></pre>
        </details>
      </section>
    </Layouts.app>
    """
  end

  attr :label, :string, required: true
  attr :value, :any, required: true

  defp metric(assigns) do
    ~H"""
    <div class="metric"><small>{@label}</small><strong>{@value}</strong></div>
    """
  end

  defp refresh(socket) do
    case Store.get_run(socket.assigns.current_identity, socket.assigns.run_id) do
      {:ok, run} ->
        {:noreply, assign_run(socket, run)}

      _ ->
        RunNotifier.unsubscribe(socket.assigns.run_id)

        {:noreply,
         socket
         |> put_flash(:error, "Your access to this run was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  defp refresh_now(socket) do
    case Store.get_run(socket.assigns.current_identity, socket.assigns.run_id) do
      {:ok, run} -> assign_run(socket, run)
      _ -> socket
    end
  end

  defp assign_run(socket, run) do
    socket
    |> assign(:run, run)
    |> assign(:patch, artifact_preview(run, "patch"))
    |> assign(:manifest, artifact_preview(run, "manifest"))
    |> stream(:events, Enum.reverse(run["events"]), reset: true)
    |> stream(:artifacts, run["artifacts"], reset: true)
  end

  defp artifact_preview(run, kind) do
    with artifact when not is_nil(artifact) <-
           Enum.find(run["artifacts"], &(&1["kind"] == kind)),
         {:ok, contents} <- ArtifactStore.read_preview(artifact["storage_key"]) do
      contents
    else
      _ -> nil
    end
  end

  defp artifact_size(run, kind) do
    case Enum.find(run["artifacts"], &(&1["kind"] == kind)) do
      nil -> "pending"
      artifact -> format_bytes(artifact["size_bytes"])
    end
  end

  defp short_sha(nil), do: "pending"
  defp short_sha(value), do: String.slice(value, 0, 10)

  defp human_state(value), do: value |> String.replace("_", " ") |> String.capitalize()
  defp active?(run), do: run["state"] in ~w(queued leasing_volume provisioning starting running)

  defp state_class(%{"outcome" => "succeeded"}), do: "success"
  defp state_class(%{"outcome" => "failed"}), do: "failure"
  defp state_class(%{"outcome" => "cancelled"}), do: "muted"
  defp state_class(_run), do: "running"

  defp proposal_tone("approved"), do: "success"
  defp proposal_tone("rejected"), do: "danger"
  defp proposal_tone("conflicted"), do: "danger"
  defp proposal_tone(_state), do: "warning"

  defp event_label(value), do: value |> String.replace(".", " · ") |> String.replace("_", " ")

  defp log_line(payload) do
    payload["line"] || payload["message"] || payload["data"] || Jason.encode!(payload)
  end

  defp format_bytes(nil), do: "—"
  defp format_bytes(value) when value < 1_024, do: "#{value} B"
  defp format_bytes(value) when value < 1_048_576, do: "#{Float.round(value / 1_024, 1)} KB"
  defp format_bytes(value), do: "#{Float.round(value / 1_048_576, 1)} MB"

  defp metric_value(metric) do
    labels =
      metric["labels"]
      |> Enum.sort()
      |> Enum.map_join(" ", fn {key, value} -> "#{key}=#{value}" end)

    if labels == "", do: metric["value"], else: "#{metric["value"]} · #{labels}"
  end

  defp control_message("approve_result"),
    do: "Approval queued. The host will CAS fast-forward the target."

  defp control_message("reject_result"), do: "Rejection queued."
  defp control_message("retry_run"), do: "Retry queued from the exact accepted input."
  defp control_message("cancel_run"), do: "Cancellation queued."
end
