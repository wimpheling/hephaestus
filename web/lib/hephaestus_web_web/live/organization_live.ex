defmodule HephaestusWebWeb.OrganizationLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWeb.Store

  @impl true
  def mount(_params, _session, socket) do
    case Store.list_organizations(socket.assigns.current_identity) do
      {:ok, organizations} ->
        {:ok,
         socket
         |> stream_configure(:organizations,
           dom_id: &"organization-stream-#{&1["id"]}"
         )
         |> assign(:page_title, "Organizations")
         |> assign(:organization_count, length(organizations))
         |> stream(:organizations, organizations)}

      {:error, _reason} ->
        {:ok,
         socket
         |> stream_configure(:organizations,
           dom_id: &"organization-stream-#{&1["id"]}"
         )
         |> put_flash(:error, "Unable to load organizations.")
         |> assign(:organization_count, 0)
         |> stream(:organizations, [])}
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <section class="hero-panel">
        <div>
          <p class="eyebrow">Control plane</p>
          <h1>Good evening, {@current_identity.display_name}.</h1>
          <p class="lede">Review live agents, inspect exact commits, and decide what reaches Git.</p>
        </div>
        <div class="system-health">
          <span class="status-dot"></span>
          <div><strong>Forge online</strong><small>durable workers connected</small></div>
        </div>
      </section>

      <section class="section-heading">
        <div>
          <p class="eyebrow">Your perimeter</p>
          <h2>Organizations</h2>
        </div>
        <span class="count-pill">{@organization_count} visible</span>
      </section>

      <div id="organizations" class="org-grid" phx-update="stream">
        <.link
          :for={{dom_id, organization} <- @streams.organizations}
          id={dom_id}
          navigate={~p"/organizations/#{organization["id"]}"}
          class="org-card"
          data-testid={"organization-#{organization["id"]}"}
        >
          <div class="org-mark">{organization["name"] |> String.first() |> String.upcase()}</div>
          <div class="org-copy">
            <h3>{organization["name"]}</h3>
            <p>
              {organization["project_count"]} projects · {organization["repository_count"]} repositories
            </p>
          </div>
          <span class="arrow">↗</span>
        </.link>
      </div>
    </Layouts.app>
    """
  end
end
