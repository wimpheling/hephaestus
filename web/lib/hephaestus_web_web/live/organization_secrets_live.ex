defmodule HephaestusWebWeb.OrganizationSecretsLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationSecretsPage
  alias HephaestusWebWeb.OrganizationSecretsState
  alias HephaestusWebWeb.PageStream

  @stream_mode :none

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = OrganizationSecretsState.new(%{organization_id: organization_id})

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Secrets")

    if connected?(socket) do
      {state, [:load]} = OrganizationSecretsState.reduce(socket.assigns.page_state, {:load, 0})

      {:ok,
       socket
       |> assign(:page_state, state)
       |> PageStream.start_snapshot(OrganizationSecretsState)}
    else
      {:ok, socket}
    end
  end

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = OrganizationSecretsState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(OrganizationSecretsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def handle_event("rotate-secret", %{"rotate" => attributes}, socket),
    do: start_command(socket, :rotate, attributes, true)

  def handle_event("revoke-secret", attributes, socket),
    do: start_command(socket, :revoke, attributes, false)

  def handle_event("set-secret-enabled", attributes, socket),
    do: start_command(socket, :set_enabled, attributes, false)

  def handle_event("purge-secret", attributes, socket),
    do: start_command(socket, :purge, attributes, false)

  @impl true
  def handle_async(:command, {:ok, event = {:command_succeeded, _message, _receipt}}, socket) do
    {state, effects} = OrganizationSecretsState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> PageStream.apply_effects(OrganizationSecretsState, effects)}
  end

  def handle_async(:command, {:ok, {:failed, _reason}}, socket),
    do: {:noreply, put_flash(socket, :error, "Command was denied or failed validation.")}

  def handle_async(:command, {:exit, _reason}, socket),
    do: {:noreply, put_flash(socket, :error, "Command service is temporarily unavailable.")}

  @impl true
  def terminate(_reason, socket) do
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  @impl true
  def render(assigns) do
    presentation = OrganizationSecretsState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <OrganizationSecretsPage.organization_secrets_page
        state={@presentation.status}
        organization={@presentation.organization}
        secrets={@presentation.secrets}
        grants={@presentation.grants}
        rotate_secret_event="rotate-secret"
        revoke_secret_event="revoke-secret"
        set_secret_enabled_event="set-secret-enabled"
        purge_secret_event="purge-secret"
      />
    </Layouts.app>
    """
  end

  defp start_command(socket, command, attributes, sensitive?) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    {submitting, _effects} = OrganizationSecretsState.reduce(state, :submitting)

    operation = fn ->
      if sensitive? do
        OrganizationSecretsState.execute_sensitive(state, identity, command, attributes)
      else
        OrganizationSecretsState.execute(state, {:command, identity, command, attributes})
      end
    end

    {:noreply, socket |> assign(:page_state, submitting) |> start_async(:command, operation)}
  end
end
