defmodule HephaestusWebWeb.ProjectRepositoryImageLive do
  @moduledoc "Route adapter for one authorized repository-image preparation history."

  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagePage
  alias HephaestusWebWeb.{PageStream, ProjectRepositoryImageState}

  @stream_mode :page_scoped

  @impl true
  def mount(%{"project_id" => project_id, "image_id" => image_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectRepositoryImageState.new(%{project_id: project_id, image_id: image_id})

    socket =
      socket
      |> assign(:project_id, project_id)
      |> assign(:page_state, state)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Image")

    if connected?(socket) do
      {:ok,
       socket
       |> PageStream.start_watch(ProjectRepositoryImageState)
       |> PageStream.start_snapshot(ProjectRepositoryImageState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, ProjectRepositoryImageState, response)
    {:noreply, PageStream.apply_effects(socket, ProjectRepositoryImageState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, ProjectRepositoryImageState, result)
    {:noreply, PageStream.apply_effects(socket, ProjectRepositoryImageState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectRepositoryImageState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectRepositoryImageState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {state, effects} = ProjectRepositoryImageState.reduce(socket.assigns.page_state, :load_failed)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectRepositoryImageState, effects)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
    PageStream.cancel(socket.assigns[:watch_task])
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  @impl true
  def render(assigns) do
    presentation = ProjectRepositoryImageState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectRepositoryImagePage.project_repository_image_page
        state={@presentation.status}
        project_id={@project_id}
        image={@presentation.image}
        history={@presentation.history}
      />
    </Layouts.app>
    """
  end
end
