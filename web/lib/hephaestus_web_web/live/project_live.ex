defmodule HephaestusWebWeb.ProjectLive do
  use HephaestusWebWeb, :live_view

  import HephaestusWebWeb.ProjectComponents

  alias HephaestusWeb.{CommandClient, RunNotifier, Store}

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    socket =
      socket
      |> stream_configure(:repositories, dom_id: &"project-repository-#{&1["id"]}")
      |> stream_configure(:instances, dom_id: &"project-instance-#{&1["id"]}")
      |> stream_configure(:runs, dom_id: &"project-run-#{&1["id"]}")
      |> stream_configure(:secrets, dom_id: &"project-secret-#{&1["id"]}")
      |> assign(:project_id, project_id)
      |> assign(:item_count, 0)
      |> assign(:release_catalog, [])
      |> assign(:project_secrets, [])
      |> assign(:secret_authority, %{"grants" => [], "imports" => []})
      |> assign(:project_repositories, [])
      |> assign(:secret_form, to_form(%{}, as: :secret))
      |> assign(:grant_form, to_form(%{}, as: :grant))
      |> assign(:import_form, to_form(%{}, as: :secret_import))

    case load(socket, socket.assigns.live_action) do
      {:ok, loaded} ->
        if connected?(loaded), do: RunNotifier.subscribe_repositories()
        {:ok, loaded}

      result ->
        result
    end
  end

  @impl true
  def handle_params(_params, _uri, socket) do
    {:ok, socket} = load(socket, socket.assigns.live_action)
    {:noreply, socket}
  end

  @impl true
  def handle_info(:repository_wakeup, socket) do
    case load(socket, socket.assigns.live_action) do
      {:ok, loaded} -> {:noreply, loaded}
    end
  end

  @impl true
  def handle_event("import-agent", %{"import" => attributes}, socket) do
    with {:ok, release_agent} <-
           find_by_id(socket.assigns.release_catalog, attributes["release_agent_id"]),
         {:ok, parameters} <-
           typed_parameters(release_agent["parameter_schema"], attributes["parameters"] || %{}),
         {:ok, selected_policy} <- selected_policy(attributes, release_agent),
         {:ok, response} <-
           CommandClient.execute(socket.assigns.current_identity, "import_agent", %{
             "project_id" => socket.assigns.project_id,
             "release_agent_id" => release_agent["id"],
             "name" => attributes["name"],
             "parameters" => parameters,
             "selected_policy" => selected_policy
           }) do
      {:noreply,
       socket
       |> put_flash(:info, "Agent imported as an independent project instance.")
       |> push_navigate(
         to: ~p"/projects/#{socket.assigns.project_id}/agents/#{response["instance_id"]}"
       )}
    else
      {:error, reason} ->
        {:noreply, put_flash(socket, :error, command_error("Import", reason))}
    end
  end

  def handle_event("create-secret", %{"secret" => attributes}, socket) do
    modes = selected_values(attributes, "modes")

    result =
      CommandClient.execute(socket.assigns.current_identity, "create_secret", %{
        "owner" => %{"type" => "project", "id" => socket.assigns.project_id},
        "name" => attributes["name"],
        "allowed_delivery_modes" => modes,
        "value" => attributes["value"]
      })

    command_result(socket, result, "Secret encrypted and stored.", :settings)
  end

  def handle_event("rotate-secret", %{"rotate" => attributes}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "rotate_secret", %{
        "secret_id" => attributes["secret_id"],
        "expected_active_version_id" => attributes["active_version_id"],
        "value" => attributes["value"]
      })

    command_result(socket, result, "Secret rotated for later dispatches.", :settings)
  end

  def handle_event("revoke-secret", %{"secret_id" => secret_id}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "revoke_secret", %{
        "secret_id" => secret_id
      })

    command_result(socket, result, "Secret and downstream authority revoked.", :settings)
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

    command_result(socket, result, message, :settings)
  end

  def handle_event("purge-secret", %{"secret_id" => secret_id}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "purge_secret", %{
        "secret_id" => secret_id
      })

    command_result(socket, result, "Encrypted secret material purged.", :settings)
  end

  def handle_event("grant-secret", %{"grant" => attributes}, socket) do
    with [target_kind, target_id] <- String.split(attributes["target"], ":", parts: 2) do
      result =
        CommandClient.execute(socket.assigns.current_identity, "grant_secret", %{
          "secret_id" => attributes["secret_id"],
          "target" => %{"type" => target_kind, "id" => target_id},
          "policy" => %{
            "delivery_modes" => selected_values(attributes, "modes"),
            "phases" => selected_values(attributes, "phases"),
            "destinations" => destinations(attributes["destinations"])
          },
          "expires_at" => blank_to_nil(attributes["expires_at"])
        })

      command_result(socket, result, "Bounded secret grant offered.", :settings)
    else
      _ -> {:noreply, put_flash(socket, :error, "Choose an exact grant target.")}
    end
  end

  def handle_event("accept-secret-import", %{"secret_import" => attributes}, socket) do
    with {:ok, grant} <-
           find_by_id(socket.assigns.secret_authority["grants"], attributes["grant_id"]) do
      result =
        CommandClient.execute(socket.assigns.current_identity, "accept_secret_import", %{
          "grant_id" => grant["id"],
          "target" => %{"type" => grant["target_kind"], "id" => grant["target_id"]},
          "alias" => attributes["alias"]
        })

      command_result(socket, result, "Live secret reference accepted.", :settings)
    else
      {:error, reason} ->
        {:noreply, put_flash(socket, :error, command_error("Import", reason))}
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <.breadcrumbs id="project-breadcrumbs">
        <:item navigate={~p"/organizations"}>Organizations</:item>
        <:item navigate={~p"/organizations/#{@project["organization_id"]}"}>
          {@project["organization_name"]}
        </:item>
        <:current>{@project["name"]}</:current>
      </.breadcrumbs>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Project workspace</p>
          <h1>{@project["name"]}</h1>
          <p class="lede">
            Project-owned agent instances keep independent revisions, state, and runs.
          </p>
        </div>
        <.tag>{@item_count} visible</.tag>
      </section>

      <.project_tabs project_id={@project_id} active={@live_action} />

      <section :if={@live_action == :repositories} id="project-repositories" class="repository-table">
        <div class="table-head">
          <span>Repository</span><span>Branch</span><span>Agents</span><span>Runs</span>
        </div>
        <div id="project-repository-stream" phx-update="stream">
          <p class="hidden only:block empty-copy">No visible repositories in this project.</p>
          <.link
            :for={{dom_id, repository} <- @streams.repositories}
            id={dom_id}
            navigate={~p"/repositories/#{repository["id"]}"}
            class="repo-row"
          >
            <span class="repo-name">
              <i class="repo-icon">⌘</i>
              <span>
                <strong>{repository["name"]}</strong>
                <small>{if(repository["is_public"], do: "public", else: "private")}</small>
              </span>
            </span>
            <code>{friendly_ref(repository["default_branch"])}</code>
            <span>{repository["attachment_count"]}</span>
            <span>{repository["run_count"]}</span>
          </.link>
        </div>
      </section>

      <section :if={@live_action == :agents} id="release-agent-catalog" class="panel">
        <div class="section-heading">
          <div>
            <p class="eyebrow">Source-owned immutable releases</p>
            <h2>Import a release agent</h2>
            <p class="lede">
              Importing creates a project-owned instance. Runtime fields owned by the release
              remain immutable; use a source fork for a distinct runtime contract.
            </p>
          </div>
        </div>
        <p :if={@release_catalog == []} class="empty-copy" id="release-catalog-empty">
          No published release agents are currently authorized for this project.
        </p>
        <article
          :for={release_agent <- @release_catalog}
          id={"release-catalog-#{release_agent["id"]}"}
          class="control-card"
        >
          <header>
            <div>
              <strong>{release_agent["display_name"]}</strong>
              <small>
                {release_agent["repository_name"]} · {release_agent["release_version"]} · {short_id(
                  release_agent["source_commit"]
                )}
              </small>
            </div>
            <.tag tone="success">published</.tag>
          </header>
          <.form
            for={to_form(%{}, as: :import)}
            id={"import-agent-#{release_agent["id"]}"}
            phx-submit="import-agent"
            class="command-grid"
          >
            <.input
              type="hidden"
              id={"import-release-agent-#{release_agent["id"]}"}
              name="import[release_agent_id]"
              value={release_agent["id"]}
            />
            <.input
              id={"import-name-#{release_agent["id"]}"}
              name="import[name]"
              label="Instance name"
              value=""
              required
              autocomplete="off"
            />
            <.input
              :for={declaration <- release_agent["parameter_schema"]}
              id={"import-parameter-#{release_agent["id"]}-#{declaration["name"]}"}
              name={"import[parameters][#{declaration["name"]}]"}
              label={parameter_label(declaration)}
              type={parameter_input_type(declaration)}
              options={parameter_options(declaration)}
              value={parameter_default(declaration)}
              required={declaration["required"]}
              autocomplete="off"
            />
            <.input
              id={"import-vcpus-#{release_agent["id"]}"}
              name="import[vcpus]"
              type="number"
              label="Virtual CPUs"
              value={policy_ceiling(release_agent, "vcpus")}
              min="1"
              required
            />
            <.input
              id={"import-memory-#{release_agent["id"]}"}
              name="import[memory_mib]"
              type="number"
              label="Memory (MiB)"
              value={policy_ceiling(release_agent, "memory_mib")}
              min="1"
              required
            />
            <.input
              id={"import-network-#{release_agent["id"]}"}
              name="import[network]"
              type="select"
              label="Project network restriction"
              value={policy_ceiling(release_agent, "network")}
              options={[
                {"Disabled", "disabled"},
                {"Broker only", "broker_only"},
                {"Constrained egress", "egress"}
              ]}
            />
            <button
              id={"import-submit-#{release_agent["id"]}"}
              class="button primary"
              type="submit"
              phx-disable-with="Importing…"
            >
              Import as new instance
            </button>
          </.form>
        </article>
      </section>

      <section :if={@live_action == :agents} id="project-agents" class="repository-table">
        <div class="section-heading">
          <div>
            <p class="eyebrow">Project-owned installations</p>
            <h2>Configured instances</h2>
          </div>
        </div>
        <div class="table-head">
          <span>Instance</span><span>Release</span><span>Attachments</span><span>Runs</span>
        </div>
        <div id="project-instance-stream" phx-update="stream">
          <p class="hidden only:block empty-copy">No configured agent instances yet.</p>
          <.link
            :for={{dom_id, instance} <- @streams.instances}
            id={dom_id}
            navigate={~p"/projects/#{@project_id}/agents/#{instance["id"]}"}
            class="repo-row"
          >
            <span class="repo-name">
              <i class="repo-icon">A</i>
              <span>
                <strong>{instance["name"]}</strong>
                <small>{instance["release_agent_name"] || "invalid candidate"}</small>
              </span>
            </span>
            <span>
              <.tag tone={state_tone(instance["release_state"])}>
                {instance["release_version"] || "unresolved"}
              </.tag>
            </span>
            <span>{instance["attachment_count"]}</span>
            <span>{instance["run_count"]}</span>
          </.link>
        </div>
      </section>

      <section :if={@live_action == :runs} id="project-runs" class="repository-table">
        <div class="table-head">
          <span>Run</span><span>Target</span><span>Release</span><span>Status</span>
        </div>
        <div id="project-run-stream" phx-update="stream">
          <p class="hidden only:block empty-copy">No exact runs have been created.</p>
          <.link
            :for={{dom_id, run} <- @streams.runs}
            id={dom_id}
            navigate={~p"/runs/#{run["id"]}"}
            class="repo-row"
          >
            <span class="repo-name">
              <i class="repo-icon">R</i>
              <span>
                <strong>{run["instance_name"]}</strong>
                <small>{short_id(run["id"])}</small>
              </span>
            </span>
            <span>{run["repository_name"] || "update hook"}</span>
            <code>{run["release_version"]}</code>
            <span><.tag tone={state_tone(run["outcome"] || run["state"])}>{run["state"]}</.tag></span>
          </.link>
        </div>
      </section>

      <section :if={@live_action == :settings} id="project-settings">
        <div class="section-heading spacious">
          <div>
            <p class="eyebrow">Write-only values</p>
            <h2>Project secrets</h2>
            <p class="lede">
              Metadata is visible only with inspect authority. Stored values are never returned.
            </p>
          </div>
        </div>
        <article class="panel secret-write-panel" id="create-project-secret-panel">
          <h3>Create a project-owned secret</h3>
          <p>
            The value is sent once to the encryption boundary. It is never returned or saved in
            browser state.
          </p>
          <.form
            for={@secret_form}
            id="create-project-secret"
            phx-submit="create-secret"
            class="command-grid"
          >
            <.input
              field={@secret_form[:name]}
              label="Secret name"
              required
              autocomplete="off"
              pattern="[a-z0-9][a-z0-9_-]{0,127}"
            />
            <.input
              field={@secret_form[:value]}
              type="password"
              label="New value"
              required
              autocomplete="new-password"
            />
            <.input
              id="project-secret-modes"
              name="secret[modes][]"
              type="select"
              label="Allowed delivery modes"
              value={[]}
              options={[{"Brokered capability", "brokered"}, {"Raw guest file", "raw"}]}
              multiple
              required
            />
            <button
              id="create-project-secret-submit"
              type="submit"
              class="button primary"
              phx-disable-with="Encrypting…"
            >
              Encrypt and create
            </button>
          </.form>
        </article>
        <div id="project-secret-stream" class="repository-table" phx-update="stream">
          <p class="hidden only:block empty-copy">No visible project-owned secrets.</p>
          <article
            :for={{dom_id, secret} <- @streams.secrets}
            id={dom_id}
            class="secret-record"
          >
            <header>
              <span class="repo-name">
                <i class="repo-icon">S</i>
                <span>
                  <strong>{secret["name"]}</strong>
                  <small>value unavailable by design · version {secret["active_version_sequence"]}</small>
                </span>
              </span>
              <.tag tone={state_tone(secret["status"])}>{secret["status"]}</.tag>
            </header>
            <dl class="metadata-grid">
              <div>
                <dt>Version age</dt><dd>{version_age(secret["active_version_created_at"])}</dd>
              </div>
              <div>
                <dt>Modes</dt><dd>{Enum.join(secret["allowed_delivery_modes"], ", ")}</dd>
              </div>
              <div>
                <dt>Authority</dt><dd>
                  {secret["grant_count"]} grants · {secret["import_count"]} imports
                </dd>
              </div>
              <div>
                <dt>Bindings</dt><dd>{secret["binding_count"]} total</dd>
              </div>
              <div>
                <dt>Raw exposure</dt>
                <dd>{if(secret["has_raw_binding"], do: "guest receipt enabled", else: "none")}</dd>
              </div>
              <div>
                <dt>Last use</dt>
                <dd>{last_use(secret["last_use"])}</dd>
              </div>
            </dl>
            <div class="command-row">
              <button
                :if={
                  (secret["status"] == "active" && secret["can_revoke"]) ||
                    (secret["status"] == "disabled" && secret["can_rotate"])
                }
                id={"toggle-secret-#{secret["id"]}"}
                type="button"
                class="button secondary"
                phx-click="set-secret-enabled"
                phx-value-secret_id={secret["id"]}
                phx-value-enabled={to_string(secret["status"] == "disabled")}
              >
                {if(secret["status"] == "disabled", do: "Enable", else: "Disable")}
              </button>
              <.form
                :if={secret["can_rotate"] && secret["status"] in ["active", "disabled"]}
                for={to_form(%{}, as: :rotate)}
                id={"rotate-secret-#{secret["id"]}"}
                phx-submit="rotate-secret"
                class="inline-command"
              >
                <.input
                  type="hidden"
                  id={"rotate-secret-id-#{secret["id"]}"}
                  name="rotate[secret_id]"
                  value={secret["id"]}
                />
                <.input
                  type="hidden"
                  id={"rotate-version-id-#{secret["id"]}"}
                  name="rotate[active_version_id]"
                  value={secret["active_version_id"]}
                />
                <.input
                  id={"rotate-value-#{secret["id"]}"}
                  name="rotate[value]"
                  type="password"
                  label="Replacement value"
                  value=""
                  required
                  autocomplete="new-password"
                />
                <button
                  id={"rotate-submit-#{secret["id"]}"}
                  type="submit"
                  class="button secondary"
                  phx-disable-with="Rotating…"
                >
                  Rotate
                </button>
              </.form>
              <button
                :if={secret["can_revoke"] && secret["status"] in ["active", "disabled"]}
                id={"revoke-secret-#{secret["id"]}"}
                type="button"
                class="button danger"
                phx-click="revoke-secret"
                phx-value-secret_id={secret["id"]}
                data-confirm="Revoke the secret and all downstream grants, imports, bindings, and leases?"
              >
                Revoke
              </button>
              <button
                :if={secret["can_purge"] && secret["status"] in ["revoked", "tombstoned"]}
                id={"purge-secret-#{secret["id"]}"}
                type="button"
                class="button danger"
                phx-click="purge-secret"
                phx-value-secret_id={secret["id"]}
                data-confirm="Permanently purge encrypted material after retained leases are gone?"
              >
                Purge encrypted material
              </button>
            </div>
          </article>
        </div>

        <div class="two-column-controls">
          <article class="panel" id="grant-secret-panel">
            <h3>Offer an exact grant</h3>
            <p>Grants are non-transitive live references with explicit scope and ceilings.</p>
            <.form for={@grant_form} id="grant-secret" phx-submit="grant-secret">
              <.input
                field={@grant_form[:secret_id]}
                type="select"
                label="Owned secret"
                prompt="Choose a secret"
                options={grantable_secret_options(@project_secrets)}
                required
              />
              <.input
                field={@grant_form[:target]}
                type="select"
                label="Exact target"
                prompt="Choose a project or repository"
                options={grant_target_options(@project, @project_repositories)}
                required
              />
              <.input
                id="grant-secret-modes"
                name="grant[modes][]"
                type="select"
                label="Delivery ceiling"
                value={[]}
                options={[{"Brokered", "brokered"}, {"Raw", "raw"}]}
                multiple
                required
              />
              <.input
                id="grant-secret-phases"
                name="grant[phases][]"
                type="select"
                label="Execution phases"
                value={[]}
                options={[{"Normal runs", "normal"}, {"Update hooks", "update"}]}
                multiple
                required
              />
              <.input
                field={@grant_form[:destinations]}
                label="Broker destinations (comma-separated)"
                placeholder="api.example.com"
              />
              <.input
                field={@grant_form[:expires_at]}
                label="Expiration (RFC 3339, optional)"
                placeholder="2026-08-01T12:00:00Z"
              />
              <button id="grant-secret-submit" class="button primary" type="submit">
                Review and offer grant
              </button>
            </.form>
          </article>

          <article class="panel" id="accept-import-panel">
            <h3>Accept offered imports</h3>
            <p>
              Acceptance creates only an opaque local alias. Source rotation remains live, and
              source revocation stops later use.
            </p>
            <p :if={offered_grants(@secret_authority) == []} class="empty-copy">
              No unaccepted grants are visible.
            </p>
            <.form
              :for={grant <- offered_grants(@secret_authority)}
              for={@import_form}
              id={"accept-import-#{grant["id"]}"}
              phx-submit="accept-secret-import"
              class="inline-command"
            >
              <input type="hidden" name="secret_import[grant_id]" value={grant["id"]} />
              <p>
                <strong>{grant["secret_name"]}</strong>
                <small>
                  {grant["target_kind"]} · {Enum.join(grant["delivery_modes"], ", ")} ·
                  non-transitive
                </small>
              </p>
              <.input
                id={"import-alias-#{grant["id"]}"}
                name="secret_import[alias]"
                label="Local alias"
                value=""
                required
                autocomplete="off"
              />
              <button class="button secondary" type="submit">Accept live reference</button>
            </.form>
          </article>
        </div>
      </section>
    </Layouts.app>
    """
  end

  defp load(socket, action) do
    identity = socket.assigns.current_identity
    project_id = socket.assigns.project_id

    with {:ok, project} <- Store.get_project(identity, project_id),
         {:ok, items} <- list_for_action(identity, project_id, action) do
      socket =
        socket
        |> assign(:project, project)
        |> assign(:page_title, "#{project["name"]} · #{title(action)}")
        |> assign(:item_count, length(items))
        |> assign(:project_secrets, if(action == :settings, do: items, else: []))
        |> assign_action_context(identity, project_id, action)
        |> reset_stream(action, items)

      {:ok, socket}
    else
      {:error, _reason} ->
        {:ok,
         socket
         |> put_flash(:error, "That project is not visible.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  defp list_for_action(identity, project_id, :repositories),
    do: Store.list_project_repositories(identity, project_id)

  defp list_for_action(identity, project_id, :agents),
    do: Store.list_project_instances(identity, project_id)

  defp list_for_action(identity, project_id, :runs),
    do: Store.list_project_runs(identity, project_id)

  defp list_for_action(identity, project_id, :settings),
    do: Store.list_project_secrets(identity, project_id)

  defp reset_stream(socket, :repositories, items),
    do: stream(socket, :repositories, items, reset: true)

  defp reset_stream(socket, :agents, items), do: stream(socket, :instances, items, reset: true)
  defp reset_stream(socket, :runs, items), do: stream(socket, :runs, items, reset: true)
  defp reset_stream(socket, :settings, items), do: stream(socket, :secrets, items, reset: true)

  defp title(:repositories), do: "Repositories"
  defp title(:agents), do: "Agents"
  defp title(:runs), do: "Runs"
  defp title(:settings), do: "Settings"

  defp assign_action_context(socket, identity, project_id, :agents) do
    catalog =
      case Store.list_importable_release_agents(identity, project_id) do
        {:ok, items} -> items
        {:error, _reason} -> []
      end

    assign(socket, :release_catalog, catalog)
  end

  defp assign_action_context(socket, identity, project_id, :settings) do
    authority =
      case Store.list_project_secret_authority(identity, project_id) do
        {:ok, value} -> value
        {:error, _reason} -> %{"grants" => [], "imports" => []}
      end

    repositories =
      case Store.list_project_repositories(identity, project_id) do
        {:ok, items} -> items
        {:error, _reason} -> []
      end

    socket
    |> assign(:secret_authority, authority)
    |> assign(:project_repositories, repositories)
  end

  defp assign_action_context(socket, _identity, _project_id, _action), do: socket

  defp command_result(socket, {:ok, _response}, message, action) do
    {:ok, refreshed} = load(socket, action)
    {:noreply, put_flash(refreshed, :info, message)}
  end

  defp command_result(socket, {:error, reason}, _message, _action) do
    {:noreply, put_flash(socket, :error, command_error("Command", reason))}
  end

  defp find_by_id(items, id) do
    case Enum.find(items, &(&1["id"] == id)) do
      nil -> {:error, :unavailable}
      item -> {:ok, item}
    end
  end

  defp typed_parameters(schema, submitted) do
    Enum.reduce_while(schema, {:ok, %{}}, fn declaration, {:ok, values} ->
      name = declaration["name"]
      type = parameter_type(declaration)
      raw = submitted[name]

      case typed_value(type, raw) do
        {:ok, value} -> {:cont, {:ok, Map.put(values, name, value)}}
        :error -> {:halt, {:error, {:invalid_parameter, name}}}
      end
    end)
  end

  defp typed_value("integer", value) do
    case Integer.parse(value || "") do
      {integer, ""} -> {:ok, integer}
      _ -> :error
    end
  end

  defp typed_value("boolean", value), do: {:ok, value == "true"}
  defp typed_value(_type, value) when is_binary(value), do: {:ok, value}
  defp typed_value(_type, _value), do: :error

  defp parameter_type(declaration) do
    get_in(declaration, ["value_type", "type"]) || declaration["type"] || "string"
  end

  defp selected_policy(attributes, release_agent) do
    ceiling = release_agent["runtime_contract"]["policy_ceiling"] || %{}

    with {vcpus, ""} <- Integer.parse(attributes["vcpus"] || to_string(ceiling["vcpus"])),
         {memory, ""} <-
           Integer.parse(attributes["memory_mib"] || to_string(ceiling["memory_mib"])) do
      {:ok,
       %{
         "vcpus" => vcpus,
         "memory_mib" => memory,
         "network" => attributes["network"] || ceiling["network"] || "disabled"
       }}
    else
      _ -> {:error, :invalid_resource_selection}
    end
  end

  defp selected_values(attributes, key) do
    attributes
    |> Map.get(key, [])
    |> List.wrap()
    |> Enum.reject(&(&1 in ["", "false"]))
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

  defp command_error(action, {:invalid_parameter, name}),
    do: "#{action} rejected: parameter #{name} is invalid."

  defp command_error(action, {:rejected, _status}),
    do: "#{action} was denied or failed validation."

  defp command_error(action, {:unavailable, _reason}),
    do: "#{action} service is temporarily unavailable."

  defp command_error(action, _reason), do: "#{action} could not be completed."

  defp parameter_input_type(declaration) do
    case parameter_type(declaration) do
      "integer" -> "number"
      "boolean" -> "checkbox"
      "enum" -> "select"
      _ -> "text"
    end
  end

  defp parameter_options(declaration) do
    get_in(declaration, ["value_type", "values"]) || declaration["values"] || []
  end

  defp parameter_default(declaration) do
    case declaration["default"] do
      nil -> ""
      value -> value
    end
  end

  defp parameter_label(declaration) do
    suffix =
      cond do
        declaration["sensitive"] -> " · sensitive (redacted after submit)"
        declaration["required"] -> " · required"
        true -> " · optional"
      end

    declaration["name"] <> suffix
  end

  defp policy_ceiling(release_agent, field) do
    get_in(release_agent, ["runtime_contract", "policy_ceiling", field]) ||
      case field do
        "network" -> "disabled"
        _ -> 1
      end
  end

  defp grantable_secret_options(secrets) do
    secrets
    |> Enum.filter(&(&1["can_manage_grants"] && &1["status"] == "active"))
    |> Enum.map(&{&1["name"], &1["id"]})
  end

  defp grant_target_options(project, repositories) do
    [{"Project · #{project["name"]}", "project:#{project["id"]}"}] ++
      Enum.map(repositories, &{"Repository · #{&1["name"]}", "repository:#{&1["id"]}"})
  end

  defp offered_grants(authority) do
    Enum.filter(authority["grants"], &is_nil(&1["import_id"]))
  end

  defp version_age(nil), do: "unavailable"

  defp version_age(created_at) do
    seconds = max(DateTime.diff(DateTime.utc_now(), created_at, :second), 0)

    cond do
      seconds < 60 -> "under a minute"
      seconds < 3600 -> "#{div(seconds, 60)} minutes"
      seconds < 86_400 -> "#{div(seconds, 3600)} hours"
      true -> "#{div(seconds, 86_400)} days"
    end
  end

  defp last_use(nil), do: "never"

  defp last_use(last_use) do
    mode = last_use["delivery_mode"] || "metadata"
    "#{last_use["operation"]} · #{mode} · #{last_use["outcome"]}"
  end

  defp friendly_ref(value), do: String.replace_prefix(value, "refs/heads/", "")
  defp short_id(value), do: String.slice(value, 0, 8)

  defp state_tone(value) when value in ["published", "active", "succeeded"], do: "success"
  defp state_tone(value) when value in ["failed", "revoked", "removed"], do: "danger"
  defp state_tone(_value), do: "neutral"
end
