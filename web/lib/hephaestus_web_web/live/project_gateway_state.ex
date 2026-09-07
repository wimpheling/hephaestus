defmodule HephaestusWebWeb.ProjectGatewayState do
  @moduledoc "Gateway detail state backed by a reauthorized project event watch."

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

  defstruct status: :initial,
            data: %{
              project_id: nil,
              gateway_id: nil,
              gateway: nil,
              ingress: [],
              release_catalog: [],
              secret_authority: %{"imports" => []},
              secrets: [],
              instances: [],
              bindings: [],
              binding_form: %{}
            },
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id, gateway_id: gateway_id}),
    do: %__MODULE__{
      data: %{
        project_id: project_id,
        gateway_id: gateway_id,
        gateway: nil,
        ingress: [],
        release_catalog: [],
        secret_authority: %{"imports" => []},
        secrets: [],
        instances: [],
        bindings: [],
        binding_form: %{}
      }
    }

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode

  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:project, state.data.project_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  def reduce(state, {:watch, response}),
    do: ProductEventReducer.reduce(state, response, [:gateway_changed])

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Gateway access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, {:loaded, generation, gateway, ingress})
      when generation == state.stream_generation do
    %{state | data: %{state.data | gateway: gateway, ingress: ingress}}
    |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _generation, _gateway, _ingress}), do: {state, []}

  def reduce(state, {:loaded, generation, gateway, ingress, catalog, authority, secrets})
      when generation == state.stream_generation do
    %{
      state
      | data: %{
          state.data
          | gateway: gateway,
            ingress: ingress,
            release_catalog: catalog,
            secret_authority: authority,
            secrets: secrets
        }
    }
    |> ProductEventReducer.snapshot_complete()
  end

  def reduce(
        state,
        {:loaded, generation, gateway, ingress, catalog, authority, secrets, instances, bindings}
      )
      when generation == state.stream_generation do
    %{
      state
      | data: %{
          state.data
          | gateway: gateway,
            ingress: ingress,
            release_catalog: catalog,
            secret_authority: authority,
            secrets: secrets,
            instances: instances,
            bindings: bindings
        }
    }
    |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:configure, attributes}) do
    {%{state | status: :submitting, form: attributes, error: nil}, [{:configure, attributes}]}
  end

  def reduce(state, {:configured, generation, response})
      when generation == state.stream_generation do
    case ProductEventReducer.receipt(response) do
      {:ok, receipt} -> ProductEventReducer.await_receipt(state, receipt)
      {:error, _reason} -> {%{state | status: :stale}, [:snapshot]}
    end
  end

  def reduce(state, {:configured, _stale_generation, _response}), do: {state, []}

  def reduce(state, :configure_failed),
    do: {%{state | status: :error, error: "Gateway configuration was not accepted."}, []}

  def reduce(state, {:bind, attributes}) do
    {%{
       state
       | status: :submitting,
         data: %{state.data | binding_form: attributes},
         error: nil
     }, [{:bind, attributes}]}
  end

  def reduce(state, {:bound, generation, response})
      when generation == state.stream_generation do
    case ProductEventReducer.receipt(response) do
      {:ok, receipt} -> ProductEventReducer.await_receipt(state, receipt)
      {:error, _reason} -> {%{state | status: :stale}, [:snapshot]}
    end
  end

  def reduce(state, {:bound, _stale_generation, _response}), do: {state, []}

  def reduce(state, :bind_failed),
    do: {%{state | status: :error, error: "Mailbox binding was not accepted."}, []}

  def reduce(state, {:transitioned, response}) do
    gateway = response["gateway"] || state.data.gateway
    state = %{state | data: %{state.data | gateway: gateway}}

    case ProductEventReducer.receipt(response) do
      {:ok, receipt} -> ProductEventReducer.await_receipt(state, receipt)
      {:error, _reason} -> {%{state | status: :stale}, [:snapshot]}
    end
  end

  def reduce(state, {:lifecycle, next}) do
    {%{state | status: :submitting, error: nil}, [{:lifecycle, next}]}
  end

  def reduce(state, :lifecycle_failed) do
    {%{state | status: :error, error: "Lifecycle update was not accepted."}, []}
  end

  def execute(state, {:load, identity, generation}) do
    with {:ok, gateway} <- Client.get_gateway(identity, state.data.gateway_id),
         {:ok, ingress} <- Client.list_gateway_ingress(identity, state.data.gateway_id) do
      catalog =
        case Client.list_importable_release_agents(identity, state.data.project_id) do
          {:ok, values} -> values
          {:error, _reason} -> []
        end

      authority =
        case Client.list_project_secret_authority(identity, state.data.project_id) do
          {:ok, values} -> values
          {:error, _reason} -> %{"imports" => []}
        end

      instances =
        case Client.list_project_instances(identity, state.data.project_id) do
          {:ok, values} -> values
          {:error, _reason} -> []
        end

      bindings =
        case active_revision(gateway) do
          %{"id" => revision_id} when is_binary(revision_id) ->
            case Client.list_gateway_mailbox_bindings(identity, revision_id) do
              {:ok, values} -> values
              {:error, _reason} -> []
            end

          _ ->
            []
        end

      # GetProjectSecretAuthority includes the opaque active version for each
      # authorized import. It works for organization-owned secrets without
      # requiring project-secret-manager visibility.
      {:loaded, generation, gateway, ingress, catalog, authority, [], instances, bindings}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute(state, {:lifecycle, identity, next}) do
    gateway = state.data.gateway
    Client.set_gateway_lifecycle(identity, gateway["id"], gateway["lifecycle"], next)
  end

  def execute(state, {:configure, identity, attributes}) do
    revision = active_revision(state.data.gateway)

    catalog =
      Enum.find(state.data.release_catalog, &(&1["id"] == revision["release_agent_id"]))

    schema = (catalog && catalog["parameter_schema"]) || []

    with {:ok, parameters} <- typed_parameters(schema, attributes["parameters"] || %{}),
         {:ok, selections} <- typed_selections(attributes["secret_selections"] || %{}, state) do
      Client.configure_gateway(
        identity,
        state.data.gateway_id,
        revision["id"],
        parameters,
        selections
      )
      |> case do
        {:ok, response} -> {:configured, state.stream_generation, response}
        {:error, reason} -> {:failed, reason}
      end
    else
      {:error, reason} -> {:failed, reason}
    end
  end

  def execute(state, {:bind, identity, attributes}) do
    revision = active_revision(state.data.gateway)
    revision_id = revision["id"]
    slot_key = attributes["slot_key"]
    mailbox_id = attributes["mailbox_id"]
    producer_id = attributes["producer_id"]

    with true <- is_binary(revision_id) and revision_id != "",
         true <- slot_key in (revision["mailbox_slots"] || []),
         true <- valid_producer_id?(producer_id),
         true <- authorized_mailbox?(state.data.instances, mailbox_id),
         {:ok, response} <-
           Client.create_gateway_mailbox_binding(
             identity,
             revision_id,
             slot_key,
             mailbox_id,
             producer_id
           ) do
      {:bound, state.stream_generation, response}
    else
      _reason -> {:failed, :invalid_binding}
    end
  end

  def present(state),
    do: %{
      status: page_status(state.status),
      gateway: state.data.gateway,
      ingress: state.data.ingress,
      release_catalog: Map.get(state.data, :release_catalog, []),
      secret_authority: Map.get(state.data, :secret_authority, %{"imports" => []}),
      secrets: Map.get(state.data, :secrets, []),
      instances: Map.get(state.data, :instances, []),
      bindings: Map.get(state.data, :bindings, []),
      binding_slots: mailbox_slots(state.data.gateway),
      binding_form: Map.get(state.data, :binding_form, %{}),
      configure_form: state.form,
      lifecycle_actions: lifecycle_actions(state.data.gateway),
      error: state.error
    }

  defp lifecycle_actions(nil), do: []

  defp lifecycle_actions(%{"lifecycle" => "enabled"}),
    do: [
      %{label: "Pause", next: "paused", variant: :secondary},
      %{label: "Remove", next: "removed", variant: :danger}
    ]

  defp lifecycle_actions(%{"lifecycle" => "paused"}),
    do: [
      %{label: "Recover", next: "enabled", variant: :primary},
      %{label: "Remove", next: "removed", variant: :danger}
    ]

  defp lifecycle_actions(%{"lifecycle" => "removed"}), do: []

  defp lifecycle_actions(_gateway),
    do: [%{label: "Remove", next: "removed", variant: :danger}]

  defp page_status(:ready), do: :ready
  defp page_status(:reconnecting), do: :reconnecting
  defp page_status(status) when status in [:error, :access_revoked], do: :error
  defp page_status(_status), do: :loading

  defp active_revision(%{"active_revision_id" => active, "revisions" => revisions}) do
    Enum.find(revisions, &(Map.get(&1, "id") == active)) || %{}
  end

  defp active_revision(_gateway), do: %{}

  defp mailbox_slots(gateway), do: active_revision(gateway)["mailbox_slots"] || []

  defp authorized_mailbox?(instances, mailbox_id) when is_binary(mailbox_id) do
    Enum.any?(instances, fn instance ->
      instance["state"] not in ["removed", "REMOVED"] and instance["mailbox_id"] == mailbox_id
    end)
  end

  defp authorized_mailbox?(_instances, _mailbox_id), do: false

  defp valid_producer_id?(producer_id),
    do: is_binary(producer_id) and byte_size(producer_id) in 1..128

  defp typed_parameters(schema, submitted) do
    Enum.reduce_while(schema, {:ok, %{}}, fn declaration, {:ok, values} ->
      name = declaration["name"]
      type = get_in(declaration, ["value_type", "type"]) || declaration["type"]

      case typed_value(type, submitted[name]) do
        {:ok, value} ->
          if valid_enum?(declaration, value) do
            {:cont, {:ok, Map.put(values, name, value)}}
          else
            {:halt, {:error, {:invalid_parameter, name}}}
          end

        :skip ->
          {:cont, {:ok, values}}

        :error ->
          {:halt, {:error, {:invalid_parameter, name}}}
      end
    end)
  end

  defp typed_value(_type, nil), do: :skip

  defp typed_value("integer", value) do
    case Integer.parse(value) do
      {integer, ""} -> {:ok, integer}
      _ -> :error
    end
  end

  defp typed_value("boolean", "true"), do: {:ok, true}
  defp typed_value("boolean", "false"), do: {:ok, false}
  defp typed_value("boolean", _value), do: :error
  defp typed_value(_type, value) when is_binary(value), do: {:ok, value}
  defp typed_value(_, _), do: :error

  defp valid_enum?(%{"value_type" => %{"type" => "enum", "values" => values}}, value),
    do: value in values

  defp valid_enum?(_declaration, _value), do: true

  defp typed_selections(values, state) do
    secrets = state.data.secrets
    imports = state.data.secret_authority["imports"] || []

    values
    |> Enum.reduce_while({:ok, []}, fn {slot, encoded}, {:ok, selections} ->
      if encoded in [nil, ""] do
        {:cont, {:ok, selections}}
      else
        case String.split(encoded, "|", parts: 4) do
          [import_id, version_id, route_path, header_name] ->
            import = Enum.find(imports, &(&1["id"] == import_id))
            secret = import && Enum.find(secrets, &(&1["id"] == import["secret_id"]))
            version_id = version_id || (secret && secret["active_version_id"])

            if import && version_id && route_path != "" && header_name != "" do
              {:cont,
               {:ok,
                selections ++
                  [
                    %{
                      "slot_key" => slot,
                      "import_id" => import_id,
                      "secret_version_id" => version_id,
                      "route_path" => route_path,
                      "header_name" => header_name
                    }
                  ]}}
            else
              {:halt, {:error, {:invalid_secret, slot}}}
            end

          _ ->
            {:halt, {:error, {:invalid_secret, slot}}}
        end
      end
    end)
  end

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
