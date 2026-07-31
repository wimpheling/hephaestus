defmodule HephaestusWebWeb.OrganizationNewSecretState do
  @moduledoc "State and create effect for the organization secret form."

  alias HephaestusWeb.RPC.Client
  alias HephaestusWebWeb.ProductEventReducer

  @secret_name_pattern ~r/\A[a-z0-9][a-z0-9_-]{0,127}\z/
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
            data: %{organization_id: nil, organization: nil, existing_names: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{organization_id: organization_id}) do
    %__MODULE__{
      data: %{organization_id: organization_id, organization: nil, existing_names: []},
      form: %{}
    }
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def reduce(state, :load), do: {%{state | status: :loading, error: nil}, [:load]}
  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:loaded, organization, existing_names}) do
    data = %{state.data | organization: organization, existing_names: existing_names}
    {%{state | status: :ready, data: data, error: nil}, []}
  end

  def reduce(state, {:failed, reason}) do
    message = present_error(reason, nil)
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
      status: presentation_status(state.status),
      organization: state.data.organization,
      form: state.form,
      error: state.error
    }
  end

  def execute(state, {:load, identity}) do
    organization_id = state.data.organization_id

    with {:ok, organization} <- Client.get_organization(identity, organization_id),
         {:ok, secrets} <- Client.list_organization_secrets(identity, organization_id) do
      {:loaded, organization, Enum.map(secrets, & &1["name"])}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute_sensitive(state, identity, :create, attributes) do
    name = attributes["name"] || ""
    modes = selected_values(attributes, "modes")

    with :ok <- validate_name(name, state.data.existing_names),
         :ok <- validate_modes(modes),
         {:ok, response} <-
           Client.create_secret(
             identity,
             :organization,
             state.data.organization_id,
             name,
             modes,
             attributes["value"]
           ),
         {:ok, _receipt} <- ProductEventReducer.receipt(response) do
      {:command_succeeded, "Organization secret encrypted and stored."}
    else
      {:error, reason} -> {:failed, reason, present_error(reason, name)}
    end
  end

  defp presentation_status(status) when status in [:ready, :submitting], do: :ready
  defp presentation_status(:reconnecting), do: :reconnecting
  defp presentation_status(status) when status in [:initial, :loading, :stale], do: :loading
  defp presentation_status(_status), do: :error

  defp validate_name(name, names) do
    cond do
      not Regex.match?(@secret_name_pattern, name) -> {:error, :invalid_name}
      name in names -> {:error, :duplicate_name}
      true -> :ok
    end
  end

  defp validate_modes([]), do: {:error, :missing_modes}
  defp validate_modes(_modes), do: :ok

  defp selected_values(attributes, key),
    do: attributes |> Map.get(key, []) |> List.wrap() |> Enum.reject(&(&1 in ["", "false"]))

  defp present_error(:invalid_name, _name),
    do:
      "Secret name must start with a lowercase letter or number and use only lowercase letters, numbers, underscores, or hyphens."

  defp present_error(:duplicate_name, name),
    do: "A secret named “#{name}” already exists in this organization."

  defp present_error(:missing_modes, _name), do: "Choose at least one allowed delivery mode."

  defp present_error(%HephaestusWeb.RPC.Error{} = error, _name),
    do: HephaestusWeb.RPC.Error.present(error)

  defp present_error({:rejected, _status}, _name), do: "Command was denied or failed validation."
  defp present_error(_reason, _name), do: "Command service is temporarily unavailable."
end
