defmodule HephaestusWebWeb.ProjectRepositoryImagesState do
  @moduledoc "State for the redacted project repository-image collection."

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
            data: %{project_id: nil, images: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}),
    do: %__MODULE__{data: %{project_id: project_id, images: []}}

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:loaded, generation, images}) when generation == state.stream_generation do
    {%{state | status: :ready, data: %{state.data | images: images}, error: nil}, []}
  end

  def reduce(state, {:loaded, _generation, _images}), do: {state, []}

  def reduce(state, {:retry_started, image_id}),
    do: {%{state | status: :submitting, form: image_id, error: nil}, []}

  def reduce(state, {:retried, image_id}) do
    images =
      Enum.map(state.data.images, fn image ->
        if image["id"] == image_id,
          do: Map.merge(image, %{"status" => "producing", "failure_reason" => ""}),
          else: image
      end)

    {%{state | status: :ready, data: %{state.data | images: images}, form: nil, error: nil}, []}
  end

  def reduce(state, {:error, _reason}),
    do: {%{state | status: :error, error: "Image operation could not be completed."}, []}

  def reduce(state, :load_failed),
    do: {%{state | status: :error, error: "Image resources are unavailable."}, []}

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def execute(state, {:load, identity, generation}) do
    case Client.list_project_repository_images(identity, state.data.project_id) do
      {:ok, images} -> {:loaded, generation, images}
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute(_state, {:retry, identity, image_id}) do
    case Client.retry_project_repository_image(identity, image_id) do
      {:ok, _receipt} -> {:retried, image_id}
      {:error, reason} -> {:error, reason}
    end
  end

  def present(state),
    do: %{status: page_status(state.status), images: state.data.images, error: state.error}

  defp page_status(:ready), do: :ready
  defp page_status(:reconnecting), do: :reconnecting
  defp page_status(status) when status in [:error, :access_revoked], do: :error
  defp page_status(_status), do: :loading
end
