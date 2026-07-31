defmodule HephaestusWebWeb.OrganizationNewGrantState do
  @moduledoc "State and grant effect for the organization grant form."

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
              organization_id: nil,
              organization: nil,
              secrets: [],
              projects: [],
              repositories: []
            },
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{organization_id: organization_id}) do
    %__MODULE__{
      data: %{
        organization_id: organization_id,
        organization: nil,
        secrets: [],
        projects: [],
        repositories: []
      },
      form: %{}
    }
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def reduce(state, :load), do: {%{state | status: :loading, error: nil}, [:load]}
  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:loaded, organization, secrets, projects, repositories}) do
    data = %{
      state.data
      | organization: organization,
        secrets: secrets,
        projects: projects,
        repositories: repositories
    }

    {%{state | status: :ready, data: data, error: nil}, []}
  end

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
    Map.merge(state.data, %{
      status: presentation_status(state.status),
      form: state.form,
      error: state.error
    })
  end

  def execute(state, {:load, identity}) do
    organization_id = state.data.organization_id

    with {:ok, organization} <- Client.get_organization(identity, organization_id),
         {:ok, secrets} <- Client.list_organization_secrets(identity, organization_id),
         {:ok, projects} <- Client.list_projects(identity, organization_id),
         {:ok, repositories} <- Client.list_repositories(identity, organization_id) do
      {:loaded, organization, secrets, projects, repositories}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute(_state, {:create, identity, attributes}) do
    with [target_kind, target_id] <- String.split(attributes["target"] || "", ":", parts: 2),
         {:ok, response} <-
           Client.grant_secret(
             identity,
             attributes["secret_id"],
             target_kind,
             target_id,
             %{
               "delivery_modes" => selected_values(attributes, "modes"),
               "phases" => selected_values(attributes, "phases"),
               "destinations" => destinations(attributes["destinations"])
             },
             blank_to_nil(attributes["expires_at"])
           ),
         {:ok, _receipt} <- ProductEventReducer.receipt(response) do
      {:command_succeeded, "Exact non-transitive grant offered."}
    else
      {:error, reason} -> {:failed, reason}
      _invalid -> {:failed, :invalid_target}
    end
  end

  defp presentation_status(status) when status in [:ready, :submitting], do: :ready
  defp presentation_status(:reconnecting), do: :reconnecting
  defp presentation_status(status) when status in [:initial, :loading, :stale], do: :loading
  defp presentation_status(_status), do: :error
  defp present_error(:invalid_target), do: "Choose an exact grant target."

  defp present_error(%HephaestusWeb.RPC.Error{} = error),
    do: HephaestusWeb.RPC.Error.present(error)

  defp present_error(_reason), do: "Command service is temporarily unavailable."

  defp selected_values(attributes, key),
    do: attributes |> Map.get(key, []) |> List.wrap() |> Enum.reject(&(&1 in ["", "false"]))

  defp destinations(nil), do: []
  defp destinations(value), do: value |> String.split(",", trim: true) |> Enum.map(&String.trim/1)
  defp blank_to_nil(value) when value in [nil, ""], do: nil
  defp blank_to_nil(value), do: value
end
