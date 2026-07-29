defmodule HephaestusWebWeb.OrganizationWorkspaceLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWeb.{CommandClient, RunNotifier, Store}
  import HephaestusWebWeb.OrganizationComponents

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    if connected?(socket) do
      RunNotifier.subscribe_kinds([
        "projects",
        "repositories",
        "secrets",
        "secret_versions",
        "secret_grants",
        "secret_imports",
        "agent_secret_bindings",
        "secret_leases"
      ])
    end

    case load_workspace(socket.assigns.current_identity, organization_id) do
      {:ok, workspace} ->
        {:ok,
         socket
         |> assign(workspace)
         |> assign(:organization_id, organization_id)
         |> assign(:secret_form, to_form(%{}, as: :secret))
         |> assign(:grant_form, to_form(%{}, as: :grant))
         |> assign_page()}

      {:error, _reason} ->
        {:ok,
         socket
         |> put_flash(:error, "That organization is not visible.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def handle_params(_params, _uri, socket), do: {:noreply, assign_page(socket)}

  @impl true
  def handle_info(:repository_wakeup, socket), do: {:noreply, refresh(socket)}

  @impl true
  def handle_event("create-secret", %{"secret" => attributes}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "create_secret", %{
        "owner" => %{"type" => "organization", "id" => socket.assigns.organization_id},
        "name" => attributes["name"],
        "allowed_delivery_modes" => List.wrap(attributes["modes"]),
        "value" => attributes["value"]
      })

    command_result(
      socket,
      result,
      "Organization secret encrypted and stored.",
      :return_to_secrets
    )
  end

  def handle_event("rotate-secret", %{"rotate" => attributes}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "rotate_secret", %{
        "secret_id" => attributes["secret_id"],
        "expected_active_version_id" => attributes["active_version_id"],
        "value" => attributes["value"]
      })

    command_result(socket, result, "Organization secret rotated.")
  end

  def handle_event("revoke-secret", %{"secret_id" => secret_id}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "revoke_secret", %{
        "secret_id" => secret_id
      })

    command_result(socket, result, "Secret and downstream authority revoked.")
  end

  def handle_event("set-secret-enabled", attributes, socket) do
    enabled = attributes["enabled"] == "true"

    result =
      CommandClient.execute(socket.assigns.current_identity, "set_secret_enabled", %{
        "secret_id" => attributes["secret_id"],
        "enabled" => enabled
      })

    message =
      if enabled, do: "Later secret resolution enabled.", else: "Later resolution disabled."

    command_result(socket, result, message)
  end

  def handle_event("purge-secret", %{"secret_id" => secret_id}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "purge_secret", %{
        "secret_id" => secret_id
      })

    command_result(socket, result, "Encrypted material purged.")
  end

  def handle_event("grant-secret", %{"grant" => attributes}, socket) do
    with [target_kind, target_id] <- String.split(attributes["target"], ":", parts: 2) do
      result =
        CommandClient.execute(socket.assigns.current_identity, "grant_secret", %{
          "secret_id" => attributes["secret_id"],
          "target" => %{"type" => target_kind, "id" => target_id},
          "policy" => %{
            "delivery_modes" => List.wrap(attributes["modes"]),
            "phases" => List.wrap(attributes["phases"]),
            "destinations" => destinations(attributes["destinations"])
          },
          "expires_at" => blank_to_nil(attributes["expires_at"])
        })

      command_result(
        socket,
        result,
        "Exact non-transitive grant offered.",
        :return_to_secrets
      )
    else
      _ -> {:noreply, put_flash(socket, :error, "Choose an exact grant target.")}
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <.organization_header organization={@organization} active={active_tab(@live_action)} />

      <.projects_page :if={@live_action == :projects} projects={@projects} />
      <.secrets_page
        :if={@live_action == :secrets}
        organization={@organization}
        secrets={@secrets}
        grants={@grants}
      />
      <.new_secret_page
        :if={@live_action == :new_secret}
        organization={@organization}
        form={@secret_form}
      />
      <.new_grant_page
        :if={@live_action == :new_grant}
        organization={@organization}
        form={@grant_form}
        secrets={@secrets}
        projects={@projects}
        repositories={@repositories}
      />
    </Layouts.app>
    """
  end

  attr :projects, :list, required: true

  defp projects_page(assigns) do
    ~H"""
    <section class="section-heading workspace-heading">
      <div>
        <p class="eyebrow">Organization resources</p>
        <h2>Projects</h2>
        <p class="lede">Reusable agents, repositories, and exact runs grouped by project.</p>
      </div>
      <.tag>{length(@projects)} projects</.tag>
    </section>

    <.resource_list
      id="projects"
      columns="minmax(18rem, 1fr) 10rem 8rem 8rem"
      aria-label="Projects"
    >
      <:header>
        <span>Project</span><span>Repositories</span><span>Agents</span><span>Runs</span>
      </:header>
      <:empty :if={@projects == []}>No visible projects.</:empty>
      <:row :for={project <- @projects}>
        <.link
          id={"project-stream-#{project["id"]}"}
          navigate={~p"/projects/#{project["id"]}"}
          class="resource-list-row"
          data-testid={"project-#{project["id"]}"}
        >
          <span class="resource-primary">
            <i class="repo-icon">P</i>
            <span>
              <strong>{project["name"]}</strong>
              <small>{relative_time(project["last_activity_at"])}</small>
            </span>
          </span>
          <span>{project["repository_count"]}</span>
          <span>{project["instance_count"]}</span>
          <span>{project["run_count"]}</span>
        </.link>
      </:row>
    </.resource_list>
    """
  end

  attr :organization, :map, required: true
  attr :secrets, :list, required: true
  attr :grants, :list, required: true

  defp secrets_page(assigns) do
    ~H"""
    <section id="owned-secrets-heading" class="section-heading workspace-heading">
      <div>
        <p class="eyebrow">Organization custody</p>
        <h2>Secrets</h2>
        <p class="lede">
          Values stay write-only. Bounded grants delegate use without disclosing plaintext.
        </p>
      </div>
      <div class="page-actions">
        <.link
          navigate={~p"/organizations/#{@organization["id"]}/secrets/new"}
          class="button primary"
          data-testid="create-organization-secret-link"
        >
          <.icon name="hero-plus" class="size-4" /> Create organization secret
        </.link>
      </div>
    </section>

    <.resource_list
      id="organization-secrets"
      columns="minmax(14rem, 1fr) 7rem minmax(13rem, .8fr) minmax(22rem, 1.2fr)"
      aria-label="Owned organization secrets"
    >
      <:header>
        <span>Owned secret</span><span>Status</span><span>Authority</span><span>Controls</span>
      </:header>
      <:empty :if={@secrets == []}>No visible organization-owned secrets.</:empty>
      <:row :for={secret <- @secrets}>
        <article
          id={"organization-secret-#{secret["id"]}"}
          class="resource-list-row resource-list-row-tall"
          data-testid={"organization-secret-#{secret["id"]}"}
        >
          <span class="resource-primary">
            <i class="repo-icon"><.icon name="hero-key" class="size-5" /></i>
            <span>
              <strong>{secret["name"]}</strong>
              <small>
                version {secret["active_version_sequence"]} · value unavailable by design
              </small>
            </span>
          </span>
          <span><.tag tone={secret_tone(secret["status"])}>{secret["status"]}</.tag></span>
          <span class="resource-detail">
            <strong>{Enum.join(secret["allowed_delivery_modes"], ", ")}</strong>
            <small>
              {secret["grant_count"]} grants · {secret["import_count"]} imports · {secret[
                "binding_count"
              ]} bindings
            </small>
          </span>
          <span class="resource-controls">
            <button
              :if={
                (secret["status"] == "active" && secret["can_revoke"]) ||
                  (secret["status"] == "disabled" && secret["can_rotate"])
              }
              class="button secondary compact"
              type="button"
              phx-click="set-secret-enabled"
              phx-value-secret_id={secret["id"]}
              phx-value-enabled={to_string(secret["status"] == "disabled")}
            >
              {if(secret["status"] == "disabled", do: "Enable", else: "Disable")}
            </button>
            <.form
              :if={secret["can_rotate"] && secret["status"] in ["active", "disabled"]}
              for={to_form(%{}, as: :rotate)}
              id={"rotate-organization-secret-#{secret["id"]}"}
              phx-submit="rotate-secret"
              class="resource-inline-form"
            >
              <input type="hidden" name="rotate[secret_id]" value={secret["id"]} />
              <input
                type="hidden"
                name="rotate[active_version_id]"
                value={secret["active_version_id"]}
              />
              <input
                id={"organization-rotate-value-#{secret["id"]}"}
                name="rotate[value]"
                type="password"
                placeholder="Replacement value"
                aria-label={"Replacement value for #{secret["name"]}"}
                required
                autocomplete="new-password"
              />
              <button class="button secondary compact" type="submit">Rotate</button>
            </.form>
            <button
              :if={secret["can_revoke"] && secret["status"] in ["active", "disabled"]}
              class="button danger compact"
              type="button"
              phx-click="revoke-secret"
              phx-value-secret_id={secret["id"]}
              data-confirm="Revoke this secret and every downstream authority?"
            >
              Revoke
            </button>
            <button
              :if={secret["can_purge"] && secret["status"] in ["revoked", "tombstoned"]}
              class="button danger compact"
              type="button"
              phx-click="purge-secret"
              phx-value-secret_id={secret["id"]}
              data-confirm="Permanently purge encrypted material?"
            >
              Purge
            </button>
          </span>
        </article>
      </:row>
    </.resource_list>

    <section id="bounded-grants-heading" class="section-heading list-subheading">
      <div>
        <p class="eyebrow">Delegated authority</p>
        <h2>Bounded grants</h2>
        <p class="lede">Exact non-transitive grants offered from organization-owned secrets.</p>
      </div>
      <div class="page-actions">
        <.tag>{length(@grants)} grants</.tag>
        <.link
          navigate={~p"/organizations/#{@organization["id"]}/secret-grants/new"}
          class="button secondary"
          data-testid="offer-organization-grant-link"
        >
          <.icon name="hero-share" class="size-4" /> Offer a bounded grant
        </.link>
      </div>
    </section>

    <.resource_list
      id="organization-secret-grants"
      columns="minmax(14rem, .8fr) minmax(16rem, 1fr) minmax(18rem, 1fr) 8rem"
      aria-label="Bounded secret grants"
    >
      <:header>
        <span>Secret</span><span>Exact target</span><span>Bounds</span><span>Status</span>
      </:header>
      <:empty :if={@grants == []}>No bounded grants have been offered.</:empty>
      <:row :for={grant <- @grants}>
        <article id={"organization-secret-grant-#{grant["id"]}"} class="resource-list-row">
          <span class="resource-primary">
            <i class="repo-icon"><.icon name="hero-share" class="size-5" /></i>
            <span>
              <strong>{grant["secret_name"]}</strong>
              <small>{grant["import_count"]} accepted imports</small>
            </span>
          </span>
          <span class="resource-detail">
            <strong>{grant["target_name"] || grant["target_id"]}</strong>
            <small>{grant["target_kind"]}</small>
          </span>
          <span class="resource-detail">
            <strong>
              {Enum.join(grant["delivery_modes"], ", ")} · {Enum.join(grant["phases"], ", ")}
            </strong>
            <small>{grant_bounds(grant)}</small>
          </span>
          <span><.tag tone={grant_tone(grant["status"])}>{grant["status"]}</.tag></span>
        </article>
      </:row>
    </.resource_list>
    """
  end

  attr :organization, :map, required: true
  attr :form, :map, required: true

  defp new_secret_page(assigns) do
    ~H"""
    <section class="section-heading workspace-heading">
      <div>
        <p class="eyebrow">Organization custody</p>
        <h2>Create organization secret</h2>
        <p class="lede">The value is encrypted immediately and cannot be read back through the UI.</p>
      </div>
    </section>

    <article class="panel form-page-panel">
      <.form for={@form} id="create-organization-secret" phx-submit="create-secret">
        <.input
          field={@form[:name]}
          label="Secret name"
          required
          autocomplete="off"
          pattern="[a-z0-9][a-z0-9_-]{0,127}"
        />
        <.input
          field={@form[:value]}
          type="password"
          label="New value"
          required
          autocomplete="new-password"
        />
        <.input
          id="organization-secret-modes"
          name="secret[modes][]"
          type="select"
          label="Allowed delivery modes"
          value={[]}
          options={[{"Brokered capability", "brokered"}, {"Raw guest file", "raw"}]}
          multiple
          required
        />
        <div class="form-page-actions">
          <.link
            navigate={~p"/organizations/#{@organization["id"]}/secrets"}
            class="button secondary"
          >
            Cancel
          </.link>
          <button class="button primary" type="submit" phx-disable-with="Encrypting…">
            Encrypt and create
          </button>
        </div>
      </.form>
    </article>
    """
  end

  attr :organization, :map, required: true
  attr :form, :map, required: true
  attr :secrets, :list, required: true
  attr :projects, :list, required: true
  attr :repositories, :list, required: true

  defp new_grant_page(assigns) do
    ~H"""
    <section class="section-heading workspace-heading">
      <div>
        <p class="eyebrow">Delegated authority</p>
        <h2>Offer a bounded grant</h2>
        <p class="lede">
          Choose one exact target and a ceiling that downstream imports cannot widen.
        </p>
      </div>
    </section>

    <article class="panel form-page-panel">
      <.form for={@form} id="grant-organization-secret" phx-submit="grant-secret">
        <.input
          field={@form[:secret_id]}
          type="select"
          label="Owned secret"
          prompt="Choose a secret"
          options={grantable_options(@secrets)}
          required
        />
        <.input
          field={@form[:target]}
          type="select"
          label="Exact target"
          prompt="Choose a project or repository"
          options={target_options(@projects, @repositories)}
          required
        />
        <.input
          id="organization-grant-modes"
          name="grant[modes][]"
          type="select"
          label="Delivery ceiling"
          value={[]}
          options={[{"Brokered", "brokered"}, {"Raw", "raw"}]}
          multiple
          required
        />
        <.input
          id="organization-grant-phases"
          name="grant[phases][]"
          type="select"
          label="Execution phases"
          value={[]}
          options={[{"Normal runs", "normal"}, {"Update hooks", "update"}]}
          multiple
          required
        />
        <.input
          field={@form[:destinations]}
          label="Broker destinations"
          placeholder="api.example.com"
        />
        <.input field={@form[:expires_at]} label="Expiration (RFC 3339, optional)" />
        <div class="form-page-actions">
          <.link
            navigate={~p"/organizations/#{@organization["id"]}/secrets"}
            class="button secondary"
          >
            Cancel
          </.link>
          <button class="button primary" type="submit">Offer exact grant</button>
        </div>
      </.form>
    </article>
    """
  end

  defp load_workspace(identity, organization_id) do
    with {:ok, organization} <- Store.get_organization(identity, organization_id),
         {:ok, projects} <- Store.list_projects(identity, organization_id),
         {:ok, repositories} <- Store.list_repositories(identity, organization_id),
         {:ok, secrets} <- Store.list_organization_secrets(identity, organization_id),
         {:ok, grants} <- Store.list_organization_secret_grants(identity, organization_id) do
      {:ok,
       %{
         organization: organization,
         projects: projects,
         repositories: repositories,
         secrets: secrets,
         grants: grants
       }}
    end
  end

  defp refresh(socket) do
    case load_workspace(socket.assigns.current_identity, socket.assigns.organization_id) do
      {:ok, workspace} ->
        assign(socket, workspace)

      {:error, _reason} ->
        socket
        |> put_flash(:error, "Organization access was revoked.")
        |> push_navigate(to: ~p"/organizations")
    end
  end

  defp assign_page(socket) do
    assign(socket, :page_title, page_title(socket.assigns.live_action))
  end

  defp page_title(:projects), do: "Projects"
  defp page_title(:secrets), do: "Secrets"
  defp page_title(:new_secret), do: "Create organization secret"
  defp page_title(:new_grant), do: "Offer bounded grant"

  defp active_tab(:projects), do: :projects
  defp active_tab(_secret_action), do: :secrets

  defp command_result(socket, result, message, navigation \\ :stay)

  defp command_result(socket, {:ok, _response}, message, navigation) do
    socket =
      socket
      |> refresh()
      |> put_flash(:info, message)

    socket =
      if navigation == :return_to_secrets do
        push_navigate(
          socket,
          to: ~p"/organizations/#{socket.assigns.organization_id}/secrets"
        )
      else
        socket
      end

    {:noreply, socket}
  end

  defp command_result(socket, {:error, {:rejected, _status}}, _message, _navigation) do
    {:noreply, put_flash(socket, :error, "Command was denied or failed validation.")}
  end

  defp command_result(socket, {:error, _reason}, _message, _navigation) do
    {:noreply, put_flash(socket, :error, "Command service is temporarily unavailable.")}
  end

  defp grantable_options(secrets) do
    secrets
    |> Enum.filter(&(&1["can_manage_grants"] && &1["status"] == "active"))
    |> Enum.map(&{&1["name"], &1["id"]})
  end

  defp target_options(projects, repositories) do
    Enum.map(projects, &{"Project · #{&1["name"]}", "project:#{&1["id"]}"}) ++
      Enum.map(
        repositories,
        &{"Repository · #{&1["project_name"]}/#{&1["name"]}", "repository:#{&1["id"]}"}
      )
  end

  defp destinations(nil), do: []

  defp destinations(value) do
    value
    |> String.split(",", trim: true)
    |> Enum.map(&String.trim/1)
    |> Enum.reject(&(&1 == ""))
  end

  defp blank_to_nil(value) when value in [nil, ""], do: nil
  defp blank_to_nil(value), do: value

  defp secret_tone("active"), do: "success"
  defp secret_tone("revoked"), do: "danger"
  defp secret_tone("purged"), do: "danger"
  defp secret_tone(_status), do: "neutral"

  defp grant_tone("active"), do: "success"
  defp grant_tone("offered"), do: "accent"
  defp grant_tone("revoked"), do: "danger"
  defp grant_tone(_status), do: "neutral"

  defp grant_bounds(grant) do
    destinations =
      case grant["destinations"] do
        [] -> "no broker destinations"
        values -> Enum.join(values, ", ")
      end

    expiration =
      case grant["expires_at"] do
        nil -> "no expiration"
        value -> "expires #{Calendar.strftime(value, "%d %b %Y %H:%M UTC")}"
      end

    "#{destinations} · #{expiration}"
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
