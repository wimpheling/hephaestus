defmodule HephaestusWebWeb.ProjectRepositoryImagesLive do
  @moduledoc "Route adapter for authorized project repository-image resources."

  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagesPage
  alias HephaestusWebWeb.{PageStream, ProjectRepositoryImagesState}

  # Image browsing is a finite, authorized read like the other project tabs.
  # Retry updates its returned item directly, without a background watch.
  @stream_mode :none

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectRepositoryImagesState.new(%{project_id: project_id})

    socket =
      socket
      |> assign(:project_id, project_id)
      |> assign(:page_state, state)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Images")

    if connected?(socket) do
      {state, [:load]} = ProjectRepositoryImagesState.reduce(state, {:load, 1})

      {:ok,
       socket
       |> assign(:page_state, state)
       |> PageStream.start_snapshot(ProjectRepositoryImagesState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectRepositoryImagesState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectRepositoryImagesState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {state, effects} =
      ProjectRepositoryImagesState.reduce(socket.assigns.page_state, :load_failed)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectRepositoryImagesState, effects)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket), do: PageStream.cancel(socket.assigns[:snapshot_task])

  @impl true
  def render(assigns) do
    presentation = ProjectRepositoryImagesState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectRepositoryImagesPage.project_repository_images_page
        state={@presentation.status}
        project_id={@project_id}
        images={@presentation.images}
        image_destination={fn image_id -> ~p"/projects/#{@project_id}/images/#{image_id}" end}
        retry_event="retry-project-image"
      />
    </Layouts.app>
    """
  end

  @impl true
  def handle_event("retry-project-image", %{"value" => image_id}, socket) do
    {state, _effects} =
      ProjectRepositoryImagesState.reduce(socket.assigns.page_state, {:retry_started, image_id})

    task =
      Task.async(fn ->
        ProjectRepositoryImagesState.execute(
          state,
          {:retry, socket.assigns.current_identity, image_id}
        )
      end)

    {:noreply, assign(socket, page_state: state, snapshot_task: task)}
  end
end
