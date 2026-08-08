defmodule HephaestusWebWeb.RepositoryNewLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryNewPage
  alias HephaestusWebWeb.RepositoryNewState

  @stream_mode :none

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = RepositoryNewState.new(%{project_id: project_id})
    {state, _effects} = RepositoryNewState.reduce(state, :load)
    identity = socket.assigns.current_identity

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:page_title, "Create repository")

    if connected?(socket) do
      {:ok,
       start_async(socket, :load, fn -> RepositoryNewState.execute(state, {:load, identity}) end)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_event("create-repository", %{"repository" => attributes}, socket) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    project_id = state.data.project_id
    {submitting, _effects} = RepositoryNewState.reduce(state, :submitting)
    {submitting, _effects} = RepositoryNewState.reduce(submitting, {:form, attributes})

    {:noreply,
     socket
     |> assign(:page_state, submitting)
     |> start_async(:command, fn ->
       RepositoryNewState.execute(state, {:create, identity, project_id, attributes})
     end)}
  end

  @impl true
  def handle_async(:load, {:ok, event}, socket), do: {:noreply, reduce_event(socket, event)}

  def handle_async(:load, {:exit, reason}, socket),
    do: {:noreply, reduce_event(socket, {:failed, reason})}

  def handle_async(:command, {:ok, event}, socket), do: {:noreply, reduce_event(socket, event)}

  def handle_async(:command, {:exit, reason}, socket),
    do: {:noreply, reduce_event(socket, {:failed, reason})}

  @impl true
  def render(assigns) do
    presentation = RepositoryNewState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      <RepositoryNewPage.repository_new
        state={@presentation.state}
        project={@presentation.project}
        form={to_form(@presentation.form, as: :repository)}
        create_event="create-repository"
      />
    </Layouts.app>
    """
  end

  defp reduce_event(socket, event) do
    {state, effects} = RepositoryNewState.reduce(socket.assigns.page_state, event)
    apply_effects(assign(socket, :page_state, state), effects)
  end

  defp apply_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:flash, kind, message}, socket -> put_flash(socket, kind, message)
      {:navigate, destination}, socket -> push_navigate(socket, to: destination)
    end)
  end
end
