defmodule HephaestusWebWeb.AgentInstanceState do
  @moduledoc "State, reducer, presentation, forms, and effects for an agent instance."

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
  @events [
    "create-attachment",
    "set-attachment",
    "remove-attachment",
    "revise-instance",
    "revise-capabilities",
    "create-update",
    "recover-update",
    "bind-secret"
  ]

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

  @doc "Declares a page-scoped wakeup stream with generation-tagged refreshes."
  @spec stream_mode() :: :page_scoped
  def stream_mode, do: @stream_mode

  @doc "Creates inert initial state and non-sensitive form models."
  @spec new(String.t()) :: t()
  def new(instance_id) do
    %__MODULE__{
      data: %{instance_id: instance_id},
      form: %{
        attachment: %{},
        revision: %{},
        update: %{},
        binding: %{},
        capabilities: %{}
      }
    }
  end

  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:agent_instance, state.data.instance_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  @doc "Purely reduces lifecycle, wakeup, interaction, and effect-result messages."
  @spec reduce(t(), term()) :: {t(), [term()]}
  def reduce(%__MODULE__{} = state, :load), do: begin_load(state, :loading)

  def reduce(%__MODULE__{} = state, {:watch, response}) do
    ProductEventReducer.reduce(state, response, [
      :agent_instance_changed,
      :repository_changed,
      :repository_ref_changed,
      :run_changed,
      :agent_secret_binding_changed
    ])
  end

  def reduce(%__MODULE__{} = state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(%__MODULE__{status: :submitting} = state, :refresh) do
    {%{state | data: Map.put(state.data, :refresh_pending, true)}, []}
  end

  def reduce(%__MODULE__{} = state, :refresh), do: begin_load(state, :stale)
  def reduce(%__MODULE__{} = state, :disconnected), do: {%{state | status: :reconnecting}, []}

  def reduce(%__MODULE__{data: %{instance: instance}} = state, :connected)
      when not is_nil(instance),
      do: {%{state | status: :ready}, []}

  def reduce(%__MODULE__{} = state, :connected), do: begin_load(state, :loading)

  def reduce(%__MODULE__{} = state, {:interaction, event, params}) when event in @events do
    generation = state.stream_generation
    params = contextualize(params, state.data[:instance])

    {%{state | status: :submitting, error: nil, stream_generation: generation},
     [{:command, generation, event, params}]}
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :loaded,
        generation,
        {:ok, instance}
      }) do
    state |> snapshot_data(instance) |> ProductEventReducer.snapshot_complete()
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :loaded,
        generation,
        {:error, _reason}
      }) do
    {%{state | status: :access_revoked, error: "That agent instance is not visible."},
     [{:navigate, "/organizations"}]}
  end

  def reduce(%__MODULE__{} = state, {:loaded, _stale_generation, _result}), do: {state, []}

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :command_completed,
        generation,
        {:ok, receipt, message}
      }) do
    {state, snapshot_effects} = ProductEventReducer.await_receipt(state, receipt)

    {state, [{:flash, :info, message} | snapshot_effects]}
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :command_completed,
        generation,
        {:error, message}
      }) do
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  def reduce(%__MODULE__{stream_generation: generation} = state, {
        :command_completed,
        generation,
        :access_revoked
      }) do
    message = "Agent access was revoked."

    {%{state | status: :access_revoked, error: message},
     [{:flash, :error, message}, {:navigate, "/organizations"}]}
  end

  def reduce(%__MODULE__{} = state, {:command_completed, _stale_generation, _result}),
    do: {state, []}

  def reduce(%__MODULE__{} = state, {:effect_failed, _reason}) do
    message = "The agent operation could not be completed."
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  @doc "Runs a reducer-issued Store or command effect outside the LiveView process."
  @spec execute(term(), map()) :: term()
  def execute({:load, generation, instance_id}, identity) do
    {:loaded, generation, Client.get_instance(identity, instance_id)}
  end

  def execute(state, {:load, identity, generation}) do
    execute({:load, generation, state.data.instance_id}, identity)
  end

  def execute({:command, generation, event, params}, identity) when event in @events do
    {:command_completed, generation, execute_command(identity, event, params)}
  end

  @doc "Builds the pure page presentation model and route destinations."
  @spec present(t()) :: map()
  def present(%__MODULE__{} = state) do
    instance = state.data[:instance]

    %{
      state: page_state(state.status),
      instance: instance,
      revisions: state.data[:revisions] || [],
      attachments: state.data[:attachments] || [],
      updates: state.data[:updates] || [],
      recent_runs: (instance && instance["recent_runs"]) || [],
      forms: state.form,
      error: state.error,
      destinations: destinations(instance)
    }
  end

  defp begin_load(state, status) when status in [:loading, :stale] do
    generation = state.stream_generation + 1

    {%{state | status: status, error: nil, stream_generation: generation},
     [{:load, generation, state.data.instance_id}]}
  end

  defp snapshot_data(state, instance) do
    data =
      Map.merge(state.data, %{
        instance_id: state.data.instance_id,
        instance: instance,
        revisions: instance["revisions"],
        attachments: instance["attachments"],
        updates: instance["updates"]
      })

    %{state | data: data}
  end

  defp page_state(status) when status in [:initial, :loading, :submitting], do: :loading
  defp page_state(:ready), do: :ready
  defp page_state(status) when status in [:stale, :reconnecting], do: :reconnecting
  defp page_state(_status), do: :error

  defp destinations(nil), do: %{}

  defp destinations(instance) do
    %{
      organization_index: "/organizations",
      organization: "/organizations/#{instance["organization_id"]}",
      project_agents: "/projects/#{instance["project_id"]}/agents",
      repositories_tab: "/projects/#{instance["project_id"]}",
      agents_tab: "/projects/#{instance["project_id"]}/agents",
      runs_tab: "/projects/#{instance["project_id"]}/runs",
      settings_tab: "/projects/#{instance["project_id"]}/settings"
    }
  end

  defp contextualize(params, instance) do
    params
    |> Map.put("instance_id", instance["id"])
    |> Map.update("attachment", %{"instance_id" => instance["id"]}, fn attributes ->
      Map.put(attributes, "instance_id", instance["id"])
    end)
    |> Map.update("revision", %{"instance_id" => instance["id"]}, fn attributes ->
      Map.put(attributes, "instance_id", instance["id"])
    end)
    |> Map.update("update", %{"instance_id" => instance["id"]}, fn attributes ->
      Map.put(attributes, "instance_id", instance["id"])
    end)
    |> Map.update("binding", %{"instance_id" => instance["id"]}, fn attributes ->
      attributes
      |> Map.put("instance_id", instance["id"])
      |> Map.put("expected_revision_id", instance["active_revision_id"])
    end)
    |> Map.update("capabilities", %{"instance_id" => instance["id"]}, fn attributes ->
      Map.put(attributes, "instance_id", instance["id"])
    end)
  end

  defp execute_command(identity, "create-attachment", %{"attachment" => attributes}) do
    selector =
      if String.ends_with?(attributes["ref_selector"], "/*") do
        %{
          "type" => "prefix",
          "value" => String.trim_trailing(attributes["ref_selector"], "/*")
        }
      else
        %{"type" => "exact", "value" => attributes["ref_selector"]}
      end

    execute_and_reload(
      identity,
      "create_attachment",
      %{
        "instance_id" => attributes["instance_id"],
        "repository_id" => attributes["repository_id"],
        "ref_selector" => selector,
        "trigger_policy" => attributes["trigger_policy"]
      },
      attributes["instance_id"],
      "Attachment created."
    )
  end

  defp execute_command(identity, "set-attachment", attributes) do
    execute_and_reload(
      identity,
      "set_attachment_enabled",
      %{
        "attachment_id" => attributes["attachment_id"],
        "enabled" => attributes["enabled"] == "true"
      },
      attributes["instance_id"],
      "Attachment lifecycle updated."
    )
  end

  defp execute_command(identity, "remove-attachment", attributes) do
    execute_and_reload(
      identity,
      "remove_attachment",
      %{
        "attachment_id" => attributes["attachment_id"]
      },
      attributes["instance_id"],
      "Attachment removed; historical provenance retained."
    )
  end

  defp execute_command(identity, "revise-instance", %{"revision" => attributes}) do
    instance_id = attributes["instance_id"]

    with {:ok, instance} <- Client.get_instance(identity, instance_id) do
      revision = active_revision(instance)

      with {:ok, parameters} <-
             typed_parameters(revision["parameter_schema"], attributes["parameters"] || %{}),
           {:ok, policy} <- selected_policy(attributes, revision) do
        execute_and_reload(
          identity,
          "revise_instance",
          %{
            "instance_id" => instance_id,
            "expected_revision_id" => revision["id"],
            "parameters" => parameters,
            "selected_policy" => policy
          },
          instance_id,
          "New immutable parameter revision activated."
        )
      else
        {:error, reason} -> {:error, command_error(reason)}
      end
    else
      {:error, _reason} -> :access_revoked
    end
  end

  defp execute_command(identity, "revise-capabilities", %{"capabilities" => attributes}) do
    instance_id = attributes["instance_id"]

    with {:ok, instance} <- Client.get_instance(identity, instance_id),
         revision <- active_revision(instance),
         requirements <- active_capability_requirements(instance, revision),
         {:ok, bindings} <-
           capability_selections(instance, requirements, attributes["slots"] || %{}) do
      execute_and_reload(
        identity,
        "revise_capabilities",
        %{
          "instance_id" => instance_id,
          "expected_revision_id" => revision["id"],
          "bindings" => bindings
        },
        instance_id,
        "Capability permissions activated in a new immutable revision."
      )
    else
      {:error, :not_found} -> :access_revoked
      {:error, reason} -> {:error, command_error(reason)}
    end
  end

  defp execute_command(identity, "create-update", %{"update" => attributes}) do
    instance_id = attributes["instance_id"]

    with {:ok, instance} <- Client.get_instance(identity, instance_id),
         current <- active_revision(instance),
         {:ok, candidate} <-
           find_by_id(instance["update_candidates"], attributes["release_agent_id"]),
         {:ok, parameters} <-
           typed_parameters(candidate["parameter_schema"], attributes["parameters"] || %{}),
         {:ok, policy} <- selected_policy(attributes, candidate) do
      execute_and_reload(
        identity,
        "create_update",
        %{
          "instance_id" => instance_id,
          "expected_revision_id" => current["id"],
          "candidate_release_agent_id" => candidate["id"],
          "parameters" => parameters,
          "selected_policy" => policy
        },
        instance_id,
        "Candidate update created and reviewed."
      )
    else
      {:error, :not_found} -> :access_revoked
      {:error, reason} -> {:error, command_error(reason)}
    end
  end

  defp execute_command(identity, "recover-update", attributes) do
    execute_and_reload(
      identity,
      "recover_update",
      %{
        "update_id" => attributes["update_id"],
        "action" => attributes["action"]
      },
      attributes["instance_id"],
      "Authorized recovery decision recorded."
    )
  end

  defp execute_command(identity, "bind-secret", %{"binding" => attributes}) do
    mode = attributes["mode"]

    if mode == "raw" && attributes["raw_confirmation"] != "true" do
      {:error, "Raw binding requires explicit confirmation that the guest can copy the value."}
    else
      execute_and_reload(
        identity,
        "bind_secret",
        %{
          "instance_id" => attributes["instance_id"],
          "expected_revision_id" => attributes["expected_revision_id"],
          "import_id" => attributes["import_id"],
          "slot" => attributes["slot"],
          "mode" => mode,
          "phases" => List.wrap(attributes["phases"]),
          "attachment_ids" => List.wrap(attributes["attachment_ids"]),
          "destinations" => split_destinations(attributes["destinations"])
        },
        attributes["instance_id"],
        "Secret binding activated in a new immutable revision."
      )
    end
  end

  defp execute_and_reload(identity, command, payload, _instance_id, message) do
    with {:ok, response} <- execute_rpc(identity, command, payload),
         {:ok, receipt} <- ProductEventReducer.receipt(response) do
      {:ok, receipt, message}
    else
      {:error, reason} -> {:error, command_error(reason)}
    end
  end

  defp execute_rpc(identity, "create_attachment", attributes) do
    Client.create_attachment(
      identity,
      attributes["instance_id"],
      attributes["repository_id"],
      attributes["ref_selector"],
      attributes["trigger_policy"]
    )
  end

  defp execute_rpc(identity, "set_attachment_enabled", attributes),
    do:
      Client.set_attachment_enabled(
        identity,
        attributes["attachment_id"],
        attributes["enabled"]
      )

  defp execute_rpc(identity, "remove_attachment", attributes),
    do: Client.remove_attachment(identity, attributes["attachment_id"])

  defp execute_rpc(identity, "revise_instance", attributes) do
    Client.revise_instance(
      identity,
      attributes["instance_id"],
      attributes["expected_revision_id"],
      attributes["parameters"],
      attributes["selected_policy"]
    )
  end

  defp execute_rpc(identity, "create_update", attributes) do
    Client.create_update(
      identity,
      attributes["instance_id"],
      attributes["expected_revision_id"],
      attributes["candidate_release_agent_id"],
      attributes["parameters"],
      attributes["selected_policy"]
    )
  end

  defp execute_rpc(identity, "recover_update", attributes),
    do: Client.recover_update(identity, attributes["update_id"], attributes["action"])

  defp execute_rpc(identity, "bind_secret", attributes),
    do: Client.bind_secret(identity, attributes)

  defp execute_rpc(identity, "revise_capabilities", attributes),
    do:
      Client.revise_capabilities(
        identity,
        attributes["instance_id"],
        attributes["expected_revision_id"],
        attributes["bindings"]
      )

  defp active_capability_requirements(instance, revision) do
    Enum.filter(
      instance["capability_requirements"] || [],
      &(&1["release_agent_id"] == revision["release_agent_id"])
    )
  end

  defp capability_selections(instance, requirements, submitted) do
    Enum.reduce_while(requirements, {:ok, []}, fn requirement, {:ok, selections} ->
      selected = submitted[requirement["slot_key"]] || %{}
      resource_id = selected["resource_id"]

      if resource_id in [nil, ""] do
        if requirement["slot_required"] do
          {:halt, {:error, {:missing_capability, requirement["slot_key"]}}}
        else
          {:cont, {:ok, selections}}
        end
      else
        option =
          Enum.find(instance["capability_resource_options"] || [], fn option ->
            option["slot_key"] == requirement["slot_key"] && option["id"] == resource_id
          end)

        optional =
          (selected["optional_operations"] || %{})
          |> Enum.filter(fn {_operation, enabled} -> enabled == "true" end)
          |> Enum.map(&elem(&1, 0))

        grantable = (option && option["grantable_operations"]) || []

        if option && Enum.all?(optional, &(&1 in grantable)) do
          binding = %{
            "slot_key" => requirement["slot_key"],
            "resource_kind" => requirement["resource_kind"],
            "resource_id" => resource_id,
            "granted_operations" => Enum.uniq(requirement["required_operations"] ++ optional)
          }

          {:cont, {:ok, [binding | selections]}}
        else
          {:halt, {:error, {:invalid_capability, requirement["slot_key"]}}}
        end
      end
    end)
  end

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
      _other -> :error
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
      _other -> {:error, :invalid_resource_selection}
    end
  end

  defp parameter_type(declaration) do
    get_in(declaration, ["value_type", "type"]) || declaration["type"] || "string"
  end

  defp split_destinations(nil), do: []

  defp split_destinations(value) do
    value
    |> String.split(",", trim: true)
    |> Enum.map(&String.trim/1)
    |> Enum.reject(&(&1 == ""))
  end

  defp command_error({:rejected, _status}), do: "Command was denied or failed validation."
  defp command_error({:unavailable, _reason}), do: "Command service is temporarily unavailable."
  defp command_error({:invalid_parameter, name}), do: "Parameter #{name} is invalid."

  defp command_error({:missing_capability, slot}),
    do: "Required capability #{slot} needs an exact resource selection."

  defp command_error({:invalid_capability, slot}),
    do: "Capability #{slot} includes an unavailable resource or permission."

  defp command_error(_reason), do: "Command could not be completed."

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
