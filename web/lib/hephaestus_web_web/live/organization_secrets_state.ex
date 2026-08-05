defmodule HephaestusWebWeb.OrganizationSecretsState do
  @moduledoc "State, commands, and backend effects for organization secrets."

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
            data: %{organization_id: nil, organization: nil, secrets: [], grants: []},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{organization_id: organization_id}) do
    %__MODULE__{
      data: %{organization_id: organization_id, organization: nil, secrets: [], grants: []},
      form: %{rotate: %{}}
    }
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode

  def reduce(state, {:load, generation}) do
    {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}
  end

  def reduce(state, {:loaded, generation, organization, secrets, grants})
      when generation == state.stream_generation do
    data = %{state.data | organization: organization, secrets: secrets, grants: grants}
    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _stale_generation, _organization, _secrets, _grants}),
    do: {state, []}

  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:command_succeeded, message, _receipt}),
    do: {%{state | status: :submitting, error: nil}, [{:flash, :info, message}, :snapshot]}

  def reduce(state, {:failed, reason}) do
    message = present_error(reason)
    {%{state | status: :error, error: message}, [{:flash, message}]}
  end

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Organization access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def present(state) do
    %{
      status: presentation_status(state),
      organization: state.data.organization,
      secrets: state.data.secrets,
      grants: state.data.grants,
      rotate_form: state.form.rotate,
      error: state.error
    }
  end

  def execute(state, {:load, identity, generation}) do
    organization_id = state.data.organization_id

    with {:ok, organization} <- Client.get_organization(identity, organization_id),
         {:ok, secrets} <- Client.list_organization_secrets(identity, organization_id),
         {:ok, grants} <- Client.list_organization_secret_grants(identity, organization_id) do
      {:loaded, generation, organization, secrets, grants}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute(state, {:command, identity, :revoke, %{"secret_id" => secret_id}}),
    do:
      execute_command(
        state,
        identity,
        "revoke_secret",
        %{"secret_id" => secret_id},
        "Secret and downstream authority revoked."
      )

  def execute(state, {:command, identity, :set_enabled, attributes}) do
    enabled = attributes["enabled"] == "true"

    message =
      if enabled, do: "Later secret resolution enabled.", else: "Later resolution disabled."

    execute_command(
      state,
      identity,
      "set_secret_enabled",
      %{
        "secret_id" => attributes["secret_id"],
        "enabled" => enabled
      },
      message
    )
  end

  def execute(state, {:command, identity, :purge, %{"secret_id" => secret_id}}),
    do:
      execute_command(
        state,
        identity,
        "purge_secret",
        %{"secret_id" => secret_id},
        "Encrypted material purged."
      )

  def execute_sensitive(state, identity, :rotate, attributes) do
    execute_command(
      state,
      identity,
      "rotate_secret",
      %{
        "secret_id" => attributes["secret_id"],
        "expected_active_version_id" => attributes["active_version_id"],
        "value" => attributes["value"]
      },
      "Organization secret rotated."
    )
  end

  defp execute_command(_state, identity, command, payload, message) do
    with {:ok, response} <- execute_rpc(identity, command, payload),
         {:ok, receipt} <- ProductEventReducer.receipt(response) do
      {:command_succeeded, message, receipt}
    else
      {:error, reason} -> {:failed, reason}
    end
  end

  defp execute_rpc(identity, "revoke_secret", attributes),
    do: Client.revoke_secret(identity, attributes["secret_id"])

  defp execute_rpc(identity, "set_secret_enabled", attributes),
    do: Client.set_secret_enabled(identity, attributes["secret_id"], attributes["enabled"])

  defp execute_rpc(identity, "purge_secret", attributes),
    do: Client.purge_secret(identity, attributes["secret_id"])

  defp execute_rpc(identity, "rotate_secret", attributes),
    do:
      Client.rotate_secret(
        identity,
        attributes["secret_id"],
        attributes["expected_active_version_id"],
        attributes["value"]
      )

  defp presentation_status(%{status: status}) when status in [:ready, :submitting], do: :ready
  defp presentation_status(%{status: :reconnecting}), do: :reconnecting

  defp presentation_status(%{status: status}) when status in [:initial, :loading, :stale],
    do: :loading

  defp presentation_status(_state), do: :error

  defp present_error(%HephaestusWeb.RPC.Error{} = error),
    do: HephaestusWeb.RPC.Error.present(error)

  defp present_error({:rejected, _status}), do: "Command was denied or failed validation."
  defp present_error(_reason), do: "Command service is temporarily unavailable."

end
