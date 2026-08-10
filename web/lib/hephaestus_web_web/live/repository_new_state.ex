defmodule HephaestusWebWeb.RepositoryNewState do
  @moduledoc "State and effects for the repository creation route."

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
            data: %{project_id: nil, project: nil},
            form: %{
              "name" => "",
              "default_branch" => "main",
              "is_public" => false,
              "agent_runs_enabled" => true
            },
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}),
    do: %__MODULE__{data: %{project_id: project_id, project: nil}}

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def reduce(state, :load), do: {%{state | status: :loading, error: nil}, [:load]}
  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}
  def reduce(state, {:form, form}), do: {%{state | form: form}, []}

  def reduce(state, {:loaded, {:ok, project}}),
    do: {%{state | status: :ready, data: %{state.data | project: project}}, []}

  def reduce(state, {:loaded, {:error, reason}}), do: reduce(state, {:access_revoked, reason})

  def reduce(state, {:created, {:ok, %{"repository_id" => repository_id}}}),
    do:
      {%{state | status: :ready, error: nil},
       [{:flash, :info, "Repository created."}, {:navigate, "/repositories/#{repository_id}"}]}

  def reduce(state, {:created, {:error, reason}}), do: reduce(state, {:failed, reason})

  def reduce(state, {:failed, reason}) do
    message = present_error(reason)
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "That project is not visible."},
       [{:flash, :error, "That project is not visible."}, {:navigate, "/organizations"}]}

  def present(state) do
    %{
      state: presentation_status(state.status),
      project: state.data.project,
      form: state.form,
      error: state.error
    }
  end

  def execute(state, {:load, identity}),
    do: {:loaded, Client.get_project(identity, state.data.project_id)}

  def execute(_state, {:create, identity, project_id, attributes}),
    do: {:created, create_repository(identity, project_id, attributes)}

  defp create_repository(identity, project_id, attributes) do
    Client.create_repository(
      identity,
      project_id,
      attributes["name"] || "",
      attributes["default_branch"] || "main",
      attributes["is_public"] == "true",
      attributes["agent_runs_enabled"] == "true"
    )
  end

  defp presentation_status(status) when status in [:ready, :submitting], do: :ready
  defp presentation_status(status) when status in [:initial, :loading, :stale], do: :loading
  defp presentation_status(:reconnecting), do: :reconnecting
  defp presentation_status(_status), do: :error

  defp present_error(%HephaestusWeb.RPC.Error{} = error),
    do: HephaestusWeb.RPC.Error.present(error)

  defp present_error(_reason), do: "Repository creation is temporarily unavailable."
end
