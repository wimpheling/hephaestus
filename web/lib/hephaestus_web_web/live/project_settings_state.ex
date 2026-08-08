defmodule HephaestusWebWeb.ProjectSettingsState do
  @moduledoc "State, forms, and command effects for project settings."

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
            data: %{
              project_id: nil,
              project: nil,
              secrets: [],
              secret_authority: %{"grants" => [], "imports" => []},
              repositories: []
            },
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}) do
    %__MODULE__{
      data: %{
        project_id: project_id,
        project: nil,
        secrets: [],
        secret_authority: %{"grants" => [], "imports" => []},
        repositories: []
      },
      form: %{secret: %{}, grant: %{}, import: %{}, rotate: %{}}
    }
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def accept_import_message, do: "Live secret reference accepted."
  def create_secret_message, do: "Secret encrypted and stored."
  def grant_message, do: "Bounded secret grant offered."

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:loaded, generation, project, secrets, authority, repositories})
      when generation == state.stream_generation do
    data = %{
      state.data
      | project: project,
        secrets: secrets,
        secret_authority: authority,
        repositories: repositories
    }

    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _generation, _project, _secrets, _authority, _repositories}),
    do: {state, []}

  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:command_succeeded, message, _receipt}),
    do: {%{state | status: :submitting, error: nil}, [{:flash, :info, message}, :snapshot]}

  def reduce(state, {:failed, reason}) do
    message = command_error("Command", reason)
    {%{state | status: :error, error: message}, [{:flash, message}]}
  end

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def present(state) do
    Map.merge(state.data, %{
      status: presentation_status(state),
      item_count: length(state.data.secrets),
      secret_form: state.form.secret,
      grant_form: state.form.grant,
      import_form: state.form.import,
      rotate_form: state.form.rotate,
      error: state.error
    })
  end

  def execute(state, {:load, identity, generation}) do
    project_id = state.data.project_id

    with {:ok, project} <- Client.get_project(identity, project_id),
         {:ok, secrets} <- Client.list_project_secrets(identity, project_id) do
      authority =
        case Client.list_project_secret_authority(identity, project_id) do
          {:ok, value} -> value
          {:error, _reason} -> %{"grants" => [], "imports" => []}
        end

      repositories =
        case Client.list_project_repositories(identity, project_id) do
          {:ok, items} -> items
          {:error, _reason} -> []
        end

      {:loaded, generation, project, secrets, authority, repositories}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute(_state, {:command, identity, :revoke, %{"secret_id" => secret_id}}),
    do:
      execute_command(
        identity,
        "revoke_secret",
        %{"secret_id" => secret_id},
        "Secret and downstream authority revoked."
      )

  def execute(_state, {:command, identity, :set_enabled, attributes}) do
    enabled = attributes["enabled"] == "true"

    message =
      if enabled, do: "Later secret resolution enabled.", else: "Later resolution disabled."

    execute_command(
      identity,
      "set_secret_enabled",
      %{
        "secret_id" => attributes["secret_id"],
        "enabled" => enabled
      },
      message
    )
  end

  def execute(_state, {:command, identity, :purge, %{"secret_id" => secret_id}}),
    do:
      execute_command(
        identity,
        "purge_secret",
        %{"secret_id" => secret_id},
        "Encrypted secret material purged."
      )

  def execute(_state, {:command, identity, :grant, attributes}) do
    with [target_kind, target_id] <- String.split(attributes["target"] || "", ":", parts: 2) do
      execute_command(
        identity,
        "grant_secret",
        %{
          "secret_id" => attributes["secret_id"],
          "target" => %{"type" => target_kind, "id" => target_id},
          "policy" => %{
            "delivery_modes" => selected_values(attributes, "modes"),
            "phases" => selected_values(attributes, "phases"),
            "destinations" => destinations(attributes["destinations"])
          },
          "expires_at" => blank_to_nil(attributes["expires_at"])
        },
        grant_message()
      )
    else
      _invalid -> {:failed, :invalid_target}
    end
  end

  def execute(state, {:command, identity, :accept_import, attributes}) do
    with {:ok, grant} <- find_by_id(state.data.secret_authority["grants"], attributes["grant_id"]) do
      execute_command(
        identity,
        "accept_secret_import",
        %{
          "grant_id" => grant["id"],
          "target" => %{"type" => grant["target_kind"], "id" => grant["target_id"]},
          "alias" => attributes["alias"]
        },
        accept_import_message()
      )
    else
      {:error, reason} -> {:failed, reason}
    end
  end

  def execute_sensitive(state, identity, :create, attributes) do
    execute_command(
      identity,
      "create_secret",
      %{
        "owner" => %{"type" => "project", "id" => state.data.project_id},
        "name" => attributes["name"],
        "allowed_delivery_modes" => selected_values(attributes, "modes"),
        "value" => attributes["value"]
      },
      create_secret_message()
    )
  end

  def execute_sensitive(_state, identity, :rotate, attributes) do
    execute_command(
      identity,
      "rotate_secret",
      %{
        "secret_id" => attributes["secret_id"],
        "expected_active_version_id" => attributes["active_version_id"],
        "value" => attributes["value"]
      },
      "Secret rotated for later dispatches."
    )
  end

  defp execute_command(identity, command, payload, message) do
    with {:ok, response} <- execute_rpc(identity, command, payload),
         {:ok, receipt} <- ProductEventReducer.receipt(response) do
      {:command_succeeded, message, receipt}
    else
      {:error, reason} -> {:failed, reason}
    end
  end

  defp execute_rpc(identity, "create_secret", attributes) do
    owner = attributes["owner"]

    Client.create_secret(
      identity,
      :project,
      owner["id"],
      attributes["name"],
      attributes["allowed_delivery_modes"],
      attributes["value"]
    )
  end

  defp execute_rpc(identity, "rotate_secret", attributes),
    do:
      Client.rotate_secret(
        identity,
        attributes["secret_id"],
        attributes["expected_active_version_id"],
        attributes["value"]
      )

  defp execute_rpc(identity, "revoke_secret", attributes),
    do: Client.revoke_secret(identity, attributes["secret_id"])

  defp execute_rpc(identity, "set_secret_enabled", attributes),
    do: Client.set_secret_enabled(identity, attributes["secret_id"], attributes["enabled"])

  defp execute_rpc(identity, "purge_secret", attributes),
    do: Client.purge_secret(identity, attributes["secret_id"])

  defp execute_rpc(identity, "grant_secret", attributes) do
    target = attributes["target"]

    Client.grant_secret(
      identity,
      attributes["secret_id"],
      target["type"],
      target["id"],
      attributes["policy"],
      attributes["expires_at"]
    )
  end

  defp execute_rpc(identity, "accept_secret_import", attributes) do
    target = attributes["target"]

    Client.accept_secret_import(
      identity,
      attributes["grant_id"],
      target["type"],
      target["id"],
      attributes["alias"]
    )
  end

  defp presentation_status(%{status: status}) when status in [:ready, :submitting], do: :ready
  defp presentation_status(%{status: :reconnecting}), do: :reconnecting

  defp presentation_status(%{status: status}) when status in [:initial, :loading, :stale],
    do: :loading

  defp presentation_status(_state), do: :error

  defp find_by_id(items, id) do
    case Enum.find(items, &(&1["id"] == id)) do
      nil -> {:error, :unavailable}
      item -> {:ok, item}
    end
  end

  defp selected_values(attributes, key),
    do: attributes |> Map.get(key, []) |> List.wrap() |> Enum.reject(&(&1 in ["", "false"]))

  defp destinations(nil), do: []
  defp destinations(value), do: value |> String.split(",", trim: true) |> Enum.map(&String.trim/1)
  defp blank_to_nil(value) when value in [nil, ""], do: nil
  defp blank_to_nil(value), do: value

  defp command_error(action, {:invalid_parameter, name}),
    do: "#{action} rejected: parameter #{name} is invalid."

  defp command_error(action, {:rejected, _status}),
    do: "#{action} was denied or failed validation."

  defp command_error(action, {:unavailable, _reason}),
    do: "#{action} service is temporarily unavailable."

  defp command_error(action, %HephaestusWeb.RPC.Error{} = error),
    do: "#{action}: #{HephaestusWeb.RPC.Error.present(error)}"

  defp command_error(action, :invalid_target), do: "#{action} requires an exact target."
  defp command_error(action, _reason), do: "#{action} could not be completed."
end
