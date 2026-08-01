defmodule HephaestusWebWeb.ProjectNewState do
  @moduledoc "State and effects for the project creation route."

  alias HephaestusWeb.RPC.Client

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
            data: %{organization_id: nil, organization: nil},
            form: %{"name" => "", "description" => ""},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{organization_id: organization_id}),
    do: %__MODULE__{data: %{organization_id: organization_id, organization: nil}}

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def reduce(state, :load), do: {%{state | status: :loading, error: nil}, [:load]}
  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:form, form}), do: {%{state | form: form}, []}

  def reduce(state, {:loaded, {:ok, organization}}),
    do: {%{state | status: :ready, data: %{state.data | organization: organization}}, []}

  def reduce(state, {:loaded, {:error, reason}}), do: reduce(state, {:access_revoked, reason})

  def reduce(state, {:created, {:ok, %{"project_id" => project_id}}}),
    do:
      {%{state | status: :ready, error: nil},
       [{:flash, :info, "Project created."}, {:navigate, "/projects/#{project_id}"}]}

  def reduce(state, {:created, {:error, reason}}), do: reduce(state, {:failed, reason})

  def reduce(state, {:failed, reason}) do
    message = present_error(reason)
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "That organization is not visible."},
       [{:flash, :error, "That organization is not visible."}, {:navigate, "/organizations"}]}

  def present(state) do
    %{
      state: presentation_status(state.status),
      organization: state.data.organization,
      form: state.form,
      error: state.error
    }
  end

  def execute(state, {:load, identity}),
    do: {:loaded, Client.get_organization(identity, state.data.organization_id)}

  def execute(_state, {:create, identity, organization_id, attributes}),
    do:
      {:created,
       Client.create_project(
         identity,
         organization_id,
         attributes["name"] || "",
         attributes["description"] || ""
       )}

  defp presentation_status(status) when status in [:ready, :submitting], do: :ready
  defp presentation_status(status) when status in [:initial, :loading, :stale], do: :loading
  defp presentation_status(:reconnecting), do: :reconnecting
  defp presentation_status(_status), do: :error

  defp present_error(%HephaestusWeb.RPC.Error{} = error),
    do: HephaestusWeb.RPC.Error.present(error)

  defp present_error(_reason), do: "Project creation is temporarily unavailable."
end
