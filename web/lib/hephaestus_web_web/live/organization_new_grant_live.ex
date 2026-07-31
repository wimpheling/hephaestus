defmodule HephaestusWebWeb.OrganizationNewGrantLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationNewGrantPage
  alias HephaestusWebWeb.OrganizationNewGrantState

  @stream_mode :none

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = OrganizationNewGrantState.new(%{organization_id: organization_id})
    {state, _effects} = OrganizationNewGrantState.reduce(state, :load)
    identity = socket.assigns.current_identity

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:page_title, "Offer bounded grant")

    if connected?(socket) do
      {:ok,
       start_async(socket, :load, fn ->
         OrganizationNewGrantState.execute(state, {:load, identity})
       end)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_event("grant-secret", %{"grant" => attributes}, socket) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    {submitting, _effects} = OrganizationNewGrantState.reduce(state, :submitting)

    {:noreply,
     socket
     |> assign(:page_state, submitting)
     |> start_async(:command, fn ->
       OrganizationNewGrantState.execute(state, {:create, identity, attributes})
     end)}
  end

  @impl true
  def handle_async(:load, {:ok, event}, socket), do: {:noreply, reduce_load(socket, event)}

  def handle_async(:load, {:exit, reason}, socket),
    do: {:noreply, reduce_load(socket, {:failed, reason})}

  def handle_async(:command, {:ok, {:command_succeeded, message}}, socket) do
    {:noreply,
     socket
     |> put_flash(:info, message)
     |> push_navigate(
       to: ~p"/organizations/#{socket.assigns.page_state.data.organization_id}/secrets"
     )}
  end

  def handle_async(:command, {:ok, {:failed, reason}}, socket) do
    message =
      if reason == :invalid_target,
        do: "Choose an exact grant target.",
        else: "Command service is temporarily unavailable."

    {:noreply, put_flash(socket, :error, message)}
  end

  def handle_async(:command, {:exit, _reason}, socket),
    do: {:noreply, put_flash(socket, :error, "Command service is temporarily unavailable.")}

  @impl true
  def render(assigns) do
    presentation = OrganizationNewGrantState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <OrganizationNewGrantPage.organization_new_grant_page
        state={@presentation.status}
        organization={@presentation.organization}
        form={@presentation.form}
        secrets={@presentation.secrets}
        projects={@presentation.projects}
        repositories={@presentation.repositories}
        grant_secret_event="grant-secret"
      />
    </Layouts.app>
    """
  end

  defp reduce_load(socket, event) do
    {state, _effects} = OrganizationNewGrantState.reduce(socket.assigns.page_state, event)
    socket = assign(socket, :page_state, state)

    if state.status == :access_revoked do
      socket
      |> put_flash(:error, "That organization is not visible.")
      |> push_navigate(to: ~p"/organizations")
    else
      socket
    end
  end
end
