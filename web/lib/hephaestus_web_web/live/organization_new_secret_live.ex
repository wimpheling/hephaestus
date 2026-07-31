defmodule HephaestusWebWeb.OrganizationNewSecretLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationNewSecretPage
  alias HephaestusWebWeb.OrganizationNewSecretState

  @stream_mode :none

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = OrganizationNewSecretState.new(%{organization_id: organization_id})
    {state, _effects} = OrganizationNewSecretState.reduce(state, :load)
    identity = socket.assigns.current_identity

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:page_title, "Create organization secret")

    if connected?(socket) do
      {:ok,
       start_async(socket, :load, fn ->
         OrganizationNewSecretState.execute(state, {:load, identity})
       end)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_event("create-secret", %{"secret" => attributes}, socket) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    {submitting, _effects} = OrganizationNewSecretState.reduce(state, :submitting)

    {:noreply,
     socket
     |> assign(:page_state, submitting)
     |> start_async(:command, fn ->
       OrganizationNewSecretState.execute_sensitive(state, identity, :create, attributes)
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

  def handle_async(:command, {:ok, {:failed, _reason, message}}, socket) do
    {:noreply, put_flash(socket, :error, message)}
  end

  def handle_async(:command, {:exit, _reason}, socket) do
    {:noreply, put_flash(socket, :error, "Command service is temporarily unavailable.")}
  end

  @impl true
  def render(assigns) do
    presentation = OrganizationNewSecretState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <OrganizationNewSecretPage.organization_new_secret_page
        state={@presentation.status}
        organization={@presentation.organization}
        form={@presentation.form}
        create_secret_event="create-secret"
      />
    </Layouts.app>
    """
  end

  defp reduce_load(socket, event) do
    {state, _effects} = OrganizationNewSecretState.reduce(socket.assigns.page_state, event)
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
