defmodule HephaestusWebWeb.ProjectNewLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectNewPage
  alias HephaestusWebWeb.ProjectNewState

  @stream_mode :none

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectNewState.new(%{organization_id: organization_id})
    {state, _effects} = ProjectNewState.reduce(state, :load)
    identity = socket.assigns.current_identity

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:page_title, "Create project")

    if connected?(socket) do
      {:ok,
       start_async(socket, :load, fn -> ProjectNewState.execute(state, {:load, identity}) end)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_event("create-project", %{"project" => attributes}, socket) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    organization_id = state.data.organization_id
    {submitting, _effects} = ProjectNewState.reduce(state, :submitting)
    {submitting, _effects} = ProjectNewState.reduce(submitting, {:form, attributes})

    {:noreply,
     socket
     |> assign(:page_state, submitting)
     |> start_async(:command, fn ->
       ProjectNewState.execute(state, {:create, identity, organization_id, attributes})
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
    presentation = ProjectNewState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      <ProjectNewPage.project_new
        state={@presentation.state}
        organization={@presentation.organization}
        form={to_form(@presentation.form, as: :project)}
        create_event="create-project"
      />
    </Layouts.app>
    """
  end

  defp reduce_event(socket, event) do
    {state, effects} = ProjectNewState.reduce(socket.assigns.page_state, event)
    apply_effects(assign(socket, :page_state, state), effects)
  end

  defp apply_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:flash, kind, message}, socket -> put_flash(socket, kind, message)
      {:navigate, destination}, socket -> push_navigate(socket, to: destination)
    end)
  end
end
