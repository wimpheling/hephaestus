defmodule HephaestusWebWeb.ProjectAgentsState do
  @moduledoc "State and effects for project agent instances and imports."

  alias HephaestusWeb.RPC.Client
  alias HephaestusWebWeb.ProductEventReducer

  @stream_mode :none
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
            data: %{project_id: nil, project: nil, instances: [], release_catalog: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}) do
    %__MODULE__{
      data: %{project_id: project_id, project: nil, instances: [], release_catalog: []},
      form: %{}
    }
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:loaded, generation, project, instances, release_catalog})
      when generation == state.stream_generation do
    data = %{
      state.data
      | project: project,
        instances: instances,
        release_catalog: release_catalog
    }

    {%{state | status: :ready, data: data, error: nil}, []}
  end

  def reduce(state, {:loaded, _generation, _project, _instances, _catalog}), do: {state, []}
  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:failed, reason}) do
    message = command_error("Import", reason)
    {%{state | status: :error, error: message}, [{:flash, message}]}
  end

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project access was revoked."},
       [{:navigate, :organizations}]}

  def present(state) do
    Map.merge(state.data, %{
      status: presentation_status(state),
      item_count: length(state.data.instances),
      import_form: state.form,
      error: state.error
    })
  end

  def execute(state, {:load, identity, generation}) do
    project_id = state.data.project_id

    with {:ok, project} <- Client.get_project(identity, project_id),
         {:ok, instances} <- Client.list_project_instances(identity, project_id) do
      catalog =
        case Client.list_importable_release_agents(identity, project_id) do
          {:ok, items} -> items
          {:error, _reason} -> []
        end

      {:loaded, generation, project, instances, catalog}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute_sensitive(state, identity, :import, attributes) do
    with {:ok, release_agent} <-
           find_by_id(state.data.release_catalog, attributes["release_agent_id"]),
         {:ok, parameters} <-
           typed_parameters(release_agent["parameter_schema"], attributes["parameters"] || %{}),
         {:ok, selected_policy} <- selected_policy(attributes, release_agent),
         {:ok, response} <-
           Client.import_agent(
             identity,
             state.data.project_id,
             release_agent["id"],
             attributes["name"],
             parameters,
             selected_policy
           ),
         {:ok, _receipt} <- ProductEventReducer.receipt(response) do
      {:command_succeeded, "Agent imported as an independent project instance.",
       response["instance_id"]}
    else
      {:error, reason} -> {:failed, reason}
    end
  end

  defp presentation_status(%{status: status}) when status in [:ready, :submitting], do: :ready

  defp presentation_status(%{status: status})
       when status in [:initial, :loading, :stale, :reconnecting],
       do: :loading

  defp presentation_status(_state), do: :error

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

  defp parameter_type(declaration),
    do: get_in(declaration, ["value_type", "type"]) || declaration["type"] || "string"

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
      _other -> {:error, :invalid_resource_selection}
    end
  end

  defp command_error(action, {:invalid_parameter, name}),
    do: "#{action} rejected: parameter #{name} is invalid."

  defp command_error(action, {:rejected, _status}),
    do: "#{action} was denied or failed validation."

  defp command_error(action, %HephaestusWeb.RPC.Error{} = error),
    do: "#{action}: #{HephaestusWeb.RPC.Error.present(error)}"

  defp command_error(action, _reason), do: "#{action} could not be completed."
end
