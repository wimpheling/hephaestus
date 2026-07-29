defmodule HephaestusWebWeb.AgentInstanceLive do
  use HephaestusWebWeb, :live_view

  import HephaestusWebWeb.ProjectComponents

  alias HephaestusWeb.{CommandClient, RunNotifier, Store}

  @impl true
  def mount(%{"instance_id" => instance_id}, _session, socket) do
    case Store.get_instance(socket.assigns.current_identity, instance_id) do
      {:ok, instance} ->
        if connected?(socket), do: RunNotifier.subscribe_repositories()

        {:ok,
         socket
         |> stream_configure(:revisions, dom_id: &"instance-revision-#{&1["id"]}")
         |> stream_configure(:attachments, dom_id: &"instance-attachment-#{&1["id"]}")
         |> stream_configure(:updates, dom_id: &"instance-update-#{&1["id"]}")
         |> assign(:page_title, "#{instance["name"]} · Agent")
         |> assign(:instance, instance)
         |> assign(:attachment_form, to_form(%{}, as: :attachment))
         |> assign(:revision_form, to_form(%{}, as: :revision))
         |> assign(:update_form, to_form(%{}, as: :update))
         |> assign(:binding_form, to_form(%{}, as: :binding))
         |> stream(:revisions, instance["revisions"])
         |> stream(:attachments, instance["attachments"])
         |> stream(:updates, instance["updates"])}

      {:error, _reason} ->
        {:ok,
         socket
         |> put_flash(:error, "That agent instance is not visible.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def handle_info(:repository_wakeup, socket) do
    case Store.get_instance(socket.assigns.current_identity, socket.assigns.instance["id"]) do
      {:ok, instance} ->
        {:noreply,
         socket
         |> assign(:instance, instance)
         |> stream(:revisions, instance["revisions"], reset: true)
         |> stream(:attachments, instance["attachments"], reset: true)
         |> stream(:updates, instance["updates"], reset: true)}

      {:error, _reason} ->
        {:noreply,
         socket
         |> put_flash(:error, "Your agent instance access was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def handle_event("create-attachment", %{"attachment" => attributes}, socket) do
    selector =
      if String.ends_with?(attributes["ref_selector"], "/*") do
        %{
          "type" => "prefix",
          "value" => String.trim_trailing(attributes["ref_selector"], "/*")
        }
      else
        %{"type" => "exact", "value" => attributes["ref_selector"]}
      end

    result =
      CommandClient.execute(socket.assigns.current_identity, "create_attachment", %{
        "instance_id" => socket.assigns.instance["id"],
        "repository_id" => attributes["repository_id"],
        "ref_selector" => selector,
        "trigger_policy" => attributes["trigger_policy"]
      })

    command_result(socket, result, "Attachment created.")
  end

  def handle_event("set-attachment", attributes, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "set_attachment_enabled", %{
        "attachment_id" => attributes["attachment_id"],
        "enabled" => attributes["enabled"] == "true"
      })

    command_result(socket, result, "Attachment lifecycle updated.")
  end

  def handle_event("remove-attachment", %{"attachment_id" => attachment_id}, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "remove_attachment", %{
        "attachment_id" => attachment_id
      })

    command_result(socket, result, "Attachment removed; historical provenance retained.")
  end

  def handle_event("revise-instance", %{"revision" => attributes}, socket) do
    revision = active_revision(socket.assigns.instance)

    with {:ok, parameters} <-
           typed_parameters(revision["parameter_schema"], attributes["parameters"] || %{}),
         {:ok, policy} <- selected_policy(attributes, revision) do
      result =
        CommandClient.execute(socket.assigns.current_identity, "revise_instance", %{
          "instance_id" => socket.assigns.instance["id"],
          "expected_revision_id" => revision["id"],
          "parameters" => parameters,
          "selected_policy" => policy
        })

      command_result(socket, result, "New immutable parameter revision activated.")
    else
      {:error, reason} ->
        {:noreply, put_flash(socket, :error, command_error(reason))}
    end
  end

  def handle_event("create-update", %{"update" => attributes}, socket) do
    current = active_revision(socket.assigns.instance)

    with {:ok, candidate} <-
           find_by_id(
             socket.assigns.instance["update_candidates"],
             attributes["release_agent_id"]
           ),
         {:ok, parameters} <-
           typed_parameters(candidate["parameter_schema"], attributes["parameters"] || %{}),
         {:ok, policy} <- selected_policy(attributes, candidate) do
      result =
        CommandClient.execute(socket.assigns.current_identity, "create_update", %{
          "instance_id" => socket.assigns.instance["id"],
          "expected_revision_id" => current["id"],
          "candidate_release_agent_id" => candidate["id"],
          "parameters" => parameters,
          "selected_policy" => policy
        })

      command_result(socket, result, "Candidate update created and reviewed.")
    else
      {:error, reason} ->
        {:noreply, put_flash(socket, :error, command_error(reason))}
    end
  end

  def handle_event("recover-update", attributes, socket) do
    result =
      CommandClient.execute(socket.assigns.current_identity, "recover_update", %{
        "update_id" => attributes["update_id"],
        "action" => attributes["action"]
      })

    command_result(socket, result, "Authorized recovery decision recorded.")
  end

  def handle_event("bind-secret", %{"binding" => attributes}, socket) do
    mode = attributes["mode"]

    if mode == "raw" && attributes["raw_confirmation"] != "true" do
      {:noreply,
       put_flash(
         socket,
         :error,
         "Raw binding requires explicit confirmation that the guest can copy the value."
       )}
    else
      result =
        CommandClient.execute(socket.assigns.current_identity, "bind_secret", %{
          "instance_id" => socket.assigns.instance["id"],
          "expected_revision_id" => socket.assigns.instance["active_revision_id"],
          "import_id" => attributes["import_id"],
          "slot" => attributes["slot"],
          "mode" => mode,
          "phases" => List.wrap(attributes["phases"]),
          "attachment_ids" => List.wrap(attributes["attachment_ids"]),
          "destinations" => destinations(attributes["destinations"])
        })

      command_result(socket, result, "Secret binding activated in a new immutable revision.")
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <.breadcrumbs id="instance-breadcrumbs">
        <:item navigate={~p"/organizations"}>Organizations</:item>
        <:item navigate={~p"/organizations/#{@instance["organization_id"]}"}>
          {@instance["organization_name"]}
        </:item>
        <:item navigate={~p"/projects/#{@instance["project_id"]}/agents"}>
          {@instance["project_name"]}
        </:item>
        <:current>{@instance["name"]}</:current>
      </.breadcrumbs>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Project agent instance</p>
          <h1>{@instance["name"]}</h1>
          <p class="lede">
            Stable installation identity with immutable revisions and exact release provenance.
          </p>
        </div>
        <div class="flex gap-2">
          <.tag tone={state_tone(@instance["state"])}>{@instance["state"]}</.tag>
          <.tag tone={if(@instance["run_gate_open"], do: "success", else: "warning")}>
            {if(@instance["run_gate_open"], do: "run gate open", else: "run gate closed")}
          </.tag>
        </div>
      </section>

      <.project_tabs project_id={@instance["project_id"]} active={:agents} />

      <section class="instance-overview" id="instance-overview">
        <article class="metric">
          <small>Active revision</small><strong>{short_id(@instance["active_revision_id"])}</strong>
        </article>
        <article class="metric">
          <small>State volume</small>
          <strong>{if(@instance["state_volume_id"], do: "healthy · retained", else: "stateless")}</strong>
        </article>
        <article class="metric">
          <small>Recent runs</small><strong>{length(@instance["recent_runs"])}</strong>
        </article>
        <article class="metric">
          <small>Update availability</small>
          <strong>{length(@instance["update_candidates"])} compatible releases</strong>
        </article>
      </section>

      <section :if={@instance["can_manage"]} class="two-column-controls">
        <article class="panel" id="revise-instance-panel">
          <h2>New parameter revision</h2>
          <p>
            Review old and candidate values. Sensitive parameters are never redisplayed and must
            be entered again.
          </p>
          <.form
            for={@revision_form}
            id="revise-instance"
            phx-submit="revise-instance"
          >
            <.input
              :for={declaration <- active_revision(@instance)["parameter_schema"]}
              id={"revision-parameter-#{declaration["name"]}"}
              name={"revision[parameters][#{declaration["name"]}]"}
              label={parameter_label(declaration, active_parameter(@instance, declaration))}
              type={parameter_input_type(declaration)}
              options={parameter_options(declaration)}
              value={active_parameter(@instance, declaration)}
              required={declaration["required"]}
              autocomplete="off"
            />
            <.input
              field={@revision_form[:vcpus]}
              type="number"
              label="Virtual CPUs"
              value={active_policy(@instance, "vcpus")}
              min="1"
              required
            />
            <.input
              field={@revision_form[:memory_mib]}
              type="number"
              label="Memory (MiB)"
              value={active_policy(@instance, "memory_mib")}
              min="1"
              required
            />
            <.input
              field={@revision_form[:network]}
              type="select"
              label="Network restriction"
              value={active_policy(@instance, "network")}
              options={[
                {"Disabled", "disabled"},
                {"Broker only", "broker_only"},
                {"Constrained egress", "egress"}
              ]}
            />
            <button class="button primary" type="submit">Review and activate revision</button>
          </.form>
        </article>

        <article class="panel" id="secret-binding-panel">
          <h2>Bind declared secret slot</h2>
          <p>
            Only live imports eligible for this project or an enabled exact attachment are listed.
          </p>
          <p
            :if={active_revision(@instance)["secret_slot_schema"] == []}
            class="empty-copy"
          >
            This release declares no secret slots.
          </p>
          <.form
            :for={slot <- active_revision(@instance)["secret_slot_schema"]}
            for={@binding_form}
            id={"bind-secret-#{slot["key"]}"}
            phx-submit="bind-secret"
            class="binding-form"
          >
            <input type="hidden" name="binding[slot]" value={slot["key"]} />
            <header>
              <strong>{slot["key"]}</strong>
              <.tag tone={if(slot["required"], do: "warning", else: "neutral")}>
                {if(slot["required"], do: "required", else: "optional")}
              </.tag>
            </header>
            <p>{slot["purpose"]}</p>
            <.input
              id={"binding-import-#{slot["key"]}"}
              name="binding[import_id]"
              type="select"
              label="Eligible opaque import"
              prompt="Choose an import"
              options={import_options(@instance["secret_imports"], slot)}
              value=""
              required
            />
            <.input
              id={"binding-mode-#{slot["key"]}"}
              name="binding[mode]"
              type="select"
              label="Delivery mode"
              options={mode_options(slot)}
              value={List.first(slot["delivery_modes"])}
              required
            />
            <.input
              id={"binding-phases-#{slot["key"]}"}
              name="binding[phases][]"
              type="select"
              label="Phases"
              options={Enum.map(slot["phases"], &{human_phase(&1), &1})}
              value={slot["phases"]}
              multiple
              required
            />
            <.input
              id={"binding-attachments-#{slot["key"]}"}
              name="binding[attachment_ids][]"
              type="select"
              label="Exact attachments"
              options={attachment_options(@instance["attachments"])}
              value={[]}
              multiple
            />
            <.input
              id={"binding-destinations-#{slot["key"]}"}
              name="binding[destinations]"
              label="Broker destinations"
              value={Enum.join(slot["destinations"], ", ")}
              readonly={slot["destinations"] != []}
            />
            <div class="danger-confirmation">
              <.input
                id={"binding-raw-confirmation-#{slot["key"]}"}
                name="binding[raw_confirmation]"
                type="checkbox"
                label="I understand raw mode gives the guest plaintext it can copy."
                value="false"
              />
            </div>
            <button class="button primary" type="submit">Create binding revision</button>
          </.form>
        </article>
      </section>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Immutable history</p>
          <h2>Revisions</h2>
        </div>
      </section>
      <div id="instance-revisions" class="repository-table" phx-update="stream">
        <p class="hidden only:block empty-copy">No visible revisions.</p>
        <article
          :for={{dom_id, revision} <- @streams.revisions}
          id={dom_id}
          class="repo-row"
        >
          <span class="repo-name">
            <i class="repo-icon">V</i>
            <span>
              <strong>{revision["release_agent_name"]}</strong>
              <small>{short_id(revision["id"])}</small>
            </span>
          </span>
          <span>
            <.tag tone={state_tone(revision["release_state"])}>
              {revision["release_version"]}
            </.tag>
          </span>
          <span>{revision["platform_policy_version"]}</span>
          <span>
            <.tag tone={if(revision["runnable"], do: "success", else: "danger")}>
              {if(revision["runnable"], do: "runnable", else: "invalid")}
            </.tag>
          </span>
        </article>
      </div>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Exact targets</p>
          <h2>Attachments</h2>
        </div>
      </section>
      <article :if={@instance["can_manage"]} class="panel" id="create-attachment-panel">
        <h3>Attach an exact target</h3>
        <.form
          for={@attachment_form}
          id="create-attachment"
          phx-submit="create-attachment"
          class="command-grid"
        >
          <.input
            field={@attachment_form[:repository_id]}
            type="select"
            label="Repository"
            prompt="Choose a repository"
            options={Enum.map(@instance["repositories"], &{&1["name"], &1["id"]})}
            required
          />
          <.input
            field={@attachment_form[:ref_selector]}
            label="Exact ref or prefix"
            value="refs/heads/main"
            required
          />
          <.input
            field={@attachment_form[:trigger_policy]}
            type="select"
            label="Trigger policy"
            value="push"
            options={[{"Push", "push"}, {"Manual", "manual"}, {"Push and manual", "push_and_manual"}]}
          />
          <button class="button primary" type="submit">Create attachment</button>
        </.form>
      </article>
      <div id="instance-attachments" class="repository-table" phx-update="stream">
        <p class="hidden only:block empty-copy">This instance has no attachments.</p>
        <article
          :for={{dom_id, attachment} <- @streams.attachments}
          id={dom_id}
          class="repo-row"
        >
          <span class="repo-name">
            <i class="repo-icon">T</i>
            <span>
              <strong>{attachment["repository_name"]}</strong>
              <small>{attachment["trigger_policy"]}</small>
            </span>
          </span>
          <code>{attachment["ref_selector"]}</code>
          <span>
            <.tag tone={attachment_tone(attachment)}>
              {attachment_state(attachment)}
            </.tag>
          </span>
          <span class="command-row">
            <button
              :if={attachment["can_manage"] && is_nil(attachment["removed_at"])}
              class="button secondary compact"
              type="button"
              phx-click="set-attachment"
              phx-value-attachment_id={attachment["id"]}
              phx-value-enabled={to_string(!attachment["enabled"])}
            >
              {if(attachment["enabled"], do: "Disable", else: "Enable")}
            </button>
            <button
              :if={attachment["can_manage"] && is_nil(attachment["removed_at"])}
              class="button danger compact"
              type="button"
              phx-click="remove-attachment"
              phx-value-attachment_id={attachment["id"]}
              data-confirm="Remove this attachment while retaining historical run provenance?"
            >
              Remove
            </button>
          </span>
        </article>
      </div>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Migration lifecycle</p>
          <h2>Updates</h2>
        </div>
      </section>
      <article
        :if={@instance["can_update"] && @instance["update_candidates"] != []}
        class="panel"
        id="create-update-panel"
      >
        <h3>Review a release update</h3>
        <p>
          Release-owned runtime changes are shown as immutable candidate provenance. Unsupported
          state-capability changes and hookless stateful migrations remain invalid with stable
          diagnostics.
        </p>
        <.form for={@update_form} id="create-update" phx-submit="create-update">
          <.input
            field={@update_form[:release_agent_id]}
            type="select"
            label="Candidate release"
            prompt="Choose a compatible release"
            options={candidate_options(@instance["update_candidates"])}
            required
          />
          <.input
            :for={declaration <- candidate_schema(@instance)}
            id={"update-parameter-#{declaration["name"]}"}
            name={"update[parameters][#{declaration["name"]}]"}
            label={parameter_label(declaration, nil)}
            type={parameter_input_type(declaration)}
            options={parameter_options(declaration)}
            value={candidate_default(@instance, declaration)}
            required={declaration["required"]}
            autocomplete="off"
          />
          <.input
            field={@update_form[:vcpus]}
            type="number"
            label="Candidate virtual CPUs"
            value={candidate_policy(@instance, "vcpus")}
            min="1"
            required
          />
          <.input
            field={@update_form[:memory_mib]}
            type="number"
            label="Candidate memory (MiB)"
            value={candidate_policy(@instance, "memory_mib")}
            min="1"
            required
          />
          <.input
            field={@update_form[:network]}
            type="select"
            label="Candidate network restriction"
            value={candidate_policy(@instance, "network")}
            options={[
              {"Disabled", "disabled"},
              {"Broker only", "broker_only"},
              {"Constrained egress", "egress"}
            ]}
          />
          <button class="button primary" type="submit">Start reviewed update</button>
        </.form>
      </article>
      <div id="instance-updates" class="repository-table" phx-update="stream">
        <p class="hidden only:block empty-copy">No release updates have been attempted.</p>
        <article :for={{dom_id, update} <- @streams.updates} id={dom_id} class="repo-row">
          <span class="repo-name">
            <i class="repo-icon">U</i>
            <span>
              <strong>{update["state"]}</strong>
              <small>{short_id(update["id"])}</small>
            </span>
          </span>
          <span>{short_id(update["expected_current_revision_id"])}</span>
          <span>{short_id(update["candidate_revision_id"])}</span>
          <span>
            <.tag tone={state_tone(update["state"])}>
              {update["final_decision"] || "pending"}
            </.tag>
            <div
              :if={update["hook_events"] != []}
              id={"update-hook-events-#{update["id"]}"}
              class="hook-events"
              aria-live="polite"
            >
              <small :for={event <- update["hook_events"]}>
                {event["sequence"]} · {event["event_type"]} · {hook_event_summary(event)}
              </small>
            </div>
            <div
              :if={@instance["can_recover"] && recovery_actions(update) != []}
              class="command-row"
            >
              <button
                :for={{label, action} <- recovery_actions(update)}
                id={"recover-#{action}-#{update["id"]}"}
                type="button"
                class="button secondary compact"
                phx-click="recover-update"
                phx-value-update_id={update["id"]}
                phx-value-action={action}
                data-confirm={"Apply authorized recovery action: #{label}?"}
              >
                {label}
              </button>
            </div>
          </span>
        </article>
      </div>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Exact provenance</p><h2>Recent runs</h2>
        </div>
      </section>
      <div id="instance-recent-runs" class="repository-table">
        <p :if={@instance["recent_runs"] == []} class="empty-copy">No runs yet.</p>
        <.link
          :for={run <- @instance["recent_runs"]}
          id={"instance-run-#{run["id"]}"}
          navigate={~p"/runs/#{run["id"]}"}
          class="repo-row"
        >
          <span>{short_id(run["id"])}</span>
          <span>{run["run_kind"]}</span>
          <span>{short_id(run["instance_revision_id"])}</span>
          <.tag tone={state_tone(run["outcome"] || run["state"])}>
            {run["outcome"] || run["state"]}
          </.tag>
        </.link>
      </div>
    </Layouts.app>
    """
  end

  defp command_result(socket, {:ok, _response}, message) do
    case Store.get_instance(socket.assigns.current_identity, socket.assigns.instance["id"]) do
      {:ok, instance} ->
        {:noreply,
         socket
         |> assign(:instance, instance)
         |> stream(:revisions, instance["revisions"], reset: true)
         |> stream(:attachments, instance["attachments"], reset: true)
         |> stream(:updates, instance["updates"], reset: true)
         |> put_flash(:info, message)}

      {:error, _reason} ->
        {:noreply,
         socket
         |> put_flash(:error, "Agent access was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  defp command_result(socket, {:error, reason}, _message) do
    {:noreply, put_flash(socket, :error, command_error(reason))}
  end

  defp command_error({:rejected, _status}), do: "Command was denied or failed validation."
  defp command_error({:unavailable, _reason}), do: "Command service is temporarily unavailable."
  defp command_error({:invalid_parameter, name}), do: "Parameter #{name} is invalid."
  defp command_error(_reason), do: "Command could not be completed."

  defp active_revision(instance) do
    Enum.find(instance["revisions"], &(&1["id"] == instance["active_revision_id"])) ||
      List.first(instance["revisions"]) ||
      %{
        "parameter_schema" => [],
        "parameters" => %{},
        "secret_slot_schema" => [],
        "resource_selection" => %{},
        "runtime_contract" => %{}
      }
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

      case typed_value(parameter_type(declaration), submitted[name]) do
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

  defp selected_policy(attributes, source) do
    ceiling = source["runtime_contract"]["policy_ceiling"] || %{}

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

  defp parameter_type(declaration) do
    get_in(declaration, ["value_type", "type"]) || declaration["type"] || "string"
  end

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

  defp parameter_label(declaration, old_value) do
    old =
      cond do
        declaration["sensitive"] -> "current: [REDACTED]"
        is_nil(old_value) or old_value == "" -> "no current value"
        true -> "current: #{old_value}"
      end

    "#{declaration["name"]} · #{old}"
  end

  defp active_parameter(instance, declaration) do
    if declaration["sensitive"] do
      ""
    else
      active_revision(instance)["parameters"][declaration["name"]] || declaration["default"] || ""
    end
  end

  defp active_policy(instance, field) do
    active_revision(instance)["resource_selection"][field] ||
      get_in(active_revision(instance), ["runtime_contract", "policy_ceiling", field]) ||
      if(field == "network", do: "disabled", else: 1)
  end

  defp import_options(imports, slot) do
    imports
    |> Enum.filter(fn secret_import ->
      Enum.any?(secret_import["delivery_modes"], &(&1 in slot["delivery_modes"]))
    end)
    |> Enum.map(fn secret_import ->
      label =
        "#{secret_import["alias"]} · #{secret_import["target_kind"]} · " <>
          Enum.join(secret_import["delivery_modes"], "/")

      {label, secret_import["id"]}
    end)
  end

  defp mode_options(slot) do
    Enum.map(slot["delivery_modes"], fn
      "raw" -> {"Raw file · guest can copy plaintext", "raw"}
      "brokered" -> {"Brokered · value withheld from guest", "brokered"}
    end)
  end

  defp attachment_options(attachments) do
    attachments
    |> Enum.filter(&(&1["enabled"] && is_nil(&1["removed_at"])))
    |> Enum.map(&{"#{&1["repository_name"]} · #{&1["ref_selector"]}", &1["id"]})
  end

  defp human_phase("normal"), do: "Normal runs"
  defp human_phase("update"), do: "Update hooks"

  defp candidate_options(candidates) do
    Enum.map(
      candidates,
      &{"#{&1["display_name"]} · #{&1["release_version"]}", &1["id"]}
    )
  end

  defp candidate_schema(instance) do
    case List.first(instance["update_candidates"]) do
      nil -> []
      candidate -> candidate["parameter_schema"]
    end
  end

  defp candidate_default(instance, declaration) do
    if declaration["sensitive"] do
      ""
    else
      active_parameter(instance, declaration)
    end
  end

  defp candidate_policy(instance, field) do
    case List.first(instance["update_candidates"]) do
      nil ->
        active_policy(instance, field)

      candidate ->
        get_in(candidate, ["runtime_contract", "policy_ceiling", field]) ||
          active_policy(instance, field)
    end
  end

  defp recovery_actions(%{"state" => state})
       when state in ["compatibility_unknown", "rejected"] do
    [{"Retry hook", "retry"}, {"Reject candidate", "reject"}]
  end

  defp recovery_actions(%{"state" => "activation_recovery"}) do
    [{"Resume activation", "resume"}]
  end

  defp recovery_actions(_update), do: []

  defp hook_event_summary(%{"event_type" => "vm.log", "payload" => payload}) do
    payload["message"] || "redacted log frame"
  end

  defp hook_event_summary(event), do: inspect(event["payload"], limit: 120, printable_limit: 120)

  defp destinations(nil), do: []

  defp destinations(value) do
    value
    |> String.split(",", trim: true)
    |> Enum.map(&String.trim/1)
    |> Enum.reject(&(&1 == ""))
  end

  defp short_id(nil), do: "—"
  defp short_id(value), do: String.slice(value, 0, 8)

  defp attachment_state(%{"removed_at" => removed_at}) when not is_nil(removed_at), do: "removed"
  defp attachment_state(%{"enabled" => true}), do: "enabled"
  defp attachment_state(_attachment), do: "disabled"

  defp attachment_tone(%{"removed_at" => removed_at}) when not is_nil(removed_at), do: "danger"
  defp attachment_tone(%{"enabled" => true}), do: "success"
  defp attachment_tone(_attachment), do: "neutral"

  defp state_tone(value) when value in ["active", "published", "activated"], do: "success"

  defp state_tone(value)
       when value in ["removed", "revoked", "compatibility_unknown", "activation_recovery"],
       do: "danger"

  defp state_tone(_value), do: "neutral"
end
