defmodule HephaestusWebWeb.RepositoryLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWeb.{RunNotifier, Store}

  @impl true
  def mount(%{"repository_id" => repository_id}, _session, socket) do
    case Store.get_repository(socket.assigns.current_identity, repository_id) do
      {:ok, repository} ->
        if connected?(socket), do: RunNotifier.subscribe_repositories()

        {:ok,
         socket
         |> stream_configure(:runs, dom_id: &"run-stream-#{&1["id"]}")
         |> assign(:page_title, repository["name"])
         |> assign_repository(repository)}

      {:error, _reason} ->
        {:ok,
         socket
         |> put_flash(:error, "Repository not found or access was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def handle_info(:repository_wakeup, socket) do
    case Store.get_repository(
           socket.assigns.current_identity,
           socket.assigns.repository["id"]
         ) do
      {:ok, repository} ->
        {:noreply, assign_repository(socket, repository)}

      _ ->
        {:noreply,
         socket
         |> put_flash(:error, "Your repository access was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <nav class="crumbs">
        <.link navigate={~p"/organizations"}>{@repository["organization_name"]}</.link>
        <span>/</span><span>{@repository["project_name"]}</span>
        <span>/</span><strong>{@repository["name"]}</strong>
      </nav>

      <section class="repo-hero">
        <div class="repo-symbol">⌘</div>
        <div>
          <p class="eyebrow">Git repository</p>
          <h1>{@repository["name"]}</h1>
          <p class="mono">{@repository["default_branch"]}</p>
        </div>
        <div class="repo-badges">
          <span class="count-pill">{if @repository["is_public"], do: "public", else: "private"}</span>
          <span class="status-live"><i></i> live</span>
        </div>
      </section>

      <section class="section-heading">
        <div>
          <p class="eyebrow">Execution history</p><h2>Agent runs</h2>
        </div>
        <span class="count-pill">{@run_count} retained</span>
      </section>

      <div id="runs" class="run-list" phx-update="stream">
        <.link
          :for={{dom_id, run} <- @streams.runs}
          id={dom_id}
          navigate={~p"/runs/#{run["id"]}"}
          class="run-row"
          data-testid={"run-#{run["id"]}"}
        >
          <span class={["run-state", state_class(run["state"], run["outcome"])]}>
            <i></i>
          </span>
          <span class="run-primary">
            <strong>{run["agent_name"]}</strong>
            <small>{short_sha(run["commit_sha"])} · {run["git_ref"]}</small>
          </span>
          <span class="run-attempt">attempt {run["attempt"]}</span>
          <span class="proposal-state">{run["proposal_state"] || human_state(run["state"])}</span>
          <span class="run-time">{Calendar.strftime(run["created_at"], "%d %b · %H:%M")}</span>
          <span class="arrow">→</span>
        </.link>
        <div id="runs-empty" class="empty-state hidden only:block">
          <strong>No runs yet</strong>
          <p>Push a commit containing a valid <code>agent.toml</code> to start one.</p>
        </div>
      </div>
    </Layouts.app>
    """
  end

  defp short_sha(value), do: String.slice(value, 0, 8)
  defp human_state(value), do: value |> String.replace("_", " ") |> String.capitalize()
  defp state_class(_state, "succeeded"), do: "success"
  defp state_class(_state, "failed"), do: "failure"
  defp state_class(_state, "cancelled"), do: "muted"
  defp state_class("cleaned_up", _outcome), do: "success"
  defp state_class(_state, _outcome), do: "running"

  defp assign_repository(socket, repository) do
    runs = repository["runs"]

    socket
    |> assign(:repository, repository)
    |> assign(:run_count, length(runs))
    |> stream(:runs, runs, reset: true)
  end
end
