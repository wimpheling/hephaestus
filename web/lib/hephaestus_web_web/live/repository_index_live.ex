defmodule HephaestusWebWeb.RepositoryIndexLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWeb.Store

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    with {:ok, organization} <-
           Store.get_organization(socket.assigns.current_identity, organization_id),
         {:ok, repositories} <-
           Store.list_repositories(socket.assigns.current_identity, organization_id) do
      {:ok,
       socket
       |> stream_configure(:repositories,
         dom_id: &"repository-stream-#{&1["id"]}"
       )
       |> assign(:page_title, "Repositories")
       |> assign(:organization, organization)
       |> assign(:organization_id, organization_id)
       |> assign(:repository_count, length(repositories))
       |> stream(:repositories, repositories)}
    else
      {:error, _reason} ->
        {:ok,
         socket
         |> put_flash(:error, "That organization is not visible.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <.breadcrumbs id="repository-index-breadcrumbs">
        <:item navigate={~p"/organizations"}>Organizations</:item>
        <:current>{@organization["name"]}</:current>
      </.breadcrumbs>
      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Organization workspace</p>
          <h1>Repositories</h1>
          <p class="lede">Live execution history from accepted Git pushes.</p>
        </div>
        <.tag>{@repository_count} repositories</.tag>
      </section>

      <div id="repositories" class="repository-table" phx-update="stream">
        <div class="table-head">
          <span>Repository</span><span>Branch</span><span>Runs</span><span>Last activity</span>
        </div>
        <.link
          :for={{dom_id, repository} <- @streams.repositories}
          id={dom_id}
          navigate={~p"/repositories/#{repository["id"]}"}
          class="repo-row"
          data-testid={"repository-#{repository["id"]}"}
        >
          <span class="repo-name">
            <i class="repo-icon">⌘</i>
            <span>
              <strong>{repository["name"]}</strong>
              <small>{repository["project_name"]}</small>
            </span>
          </span>
          <code>{String.replace_prefix(repository["default_branch"], "refs/heads/", "")}</code>
          <span>{repository["run_count"]}</span>
          <span>{relative_time(repository["last_run_at"])}</span>
        </.link>
      </div>
    </Layouts.app>
    """
  end

  defp relative_time(nil), do: "No runs yet"

  defp relative_time(value) do
    seconds = DateTime.diff(DateTime.utc_now(), value, :second)

    cond do
      seconds < 60 -> "just now"
      seconds < 3600 -> "#{div(seconds, 60)}m ago"
      seconds < 86_400 -> "#{div(seconds, 3600)}h ago"
      true -> Calendar.strftime(value, "%d %b")
    end
  end
end
