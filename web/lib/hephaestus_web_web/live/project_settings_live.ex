defmodule HephaestusWebWeb.ProjectSettingsLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectSettingsPage
  alias HephaestusWebWeb.PageStream
  alias HephaestusWebWeb.ProjectSettingsState
  @stream_mode :none

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectSettingsState.new(%{project_id: project_id})

    socket =
      socket
      |> stream_configure(:secrets, dom_id: &"project-secret-#{&1["id"]}")
      |> stream(:secrets, [])
      |> assign(:page_state, state)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Settings")

    if connected?(socket) do
      {state, [:load]} = ProjectSettingsState.reduce(socket.assigns.page_state, {:load, 0})

      {:ok,
       socket
       |> assign(:page_state, state)
       |> PageStream.start_snapshot(ProjectSettingsState)}
    else
      {:ok, socket}
    end
  end

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectSettingsState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> stream(:secrets, state.data.secrets, reset: true)
     |> PageStream.apply_effects(ProjectSettingsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def handle_event("create-secret", %{"secret" => attributes}, socket),
    do: start_command(socket, :create, attributes, true)

  def handle_event("rotate-secret", %{"rotate" => attributes}, socket),
    do: start_command(socket, :rotate, attributes, true)

  def handle_event("revoke-secret", attributes, socket),
    do: start_command(socket, :revoke, attributes, false)

  def handle_event("set-secret-enabled", attributes, socket),
    do: start_command(socket, :set_enabled, attributes, false)

  def handle_event("purge-secret", attributes, socket),
    do: start_command(socket, :purge, attributes, false)

  def handle_event("grant-secret", %{"grant" => attributes}, socket),
    do: start_command(socket, :grant, attributes, false)

  def handle_event("accept-secret-import", %{"secret_import" => attributes}, socket),
    do: start_command(socket, :accept_import, attributes, false)

  @impl true
  def handle_async(:command, {:ok, event = {:command_succeeded, _message, _receipt}}, socket) do
    {state, effects} = ProjectSettingsState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> PageStream.apply_effects(ProjectSettingsState, effects)}
  end

  def handle_async(:command, {:ok, {:failed, _reason}}, socket),
    do: {:noreply, put_flash(socket, :error, "Command was denied or failed validation.")}

  def handle_async(:command, {:exit, _reason}, socket),
    do: {:noreply, put_flash(socket, :error, "Command service is temporarily unavailable.")}

  @impl true
  def terminate(_reason, socket),
    do:
      (
        PageStream.cancel(socket.assigns[:snapshot_task])
        :ok
      )

  @impl true
  def render(assigns) do
    presentation = ProjectSettingsState.present(assigns.page_state)

    presentation =
      Map.put(presentation, :forms, %{
        secret: presentation.secret_form,
        grant: presentation.grant_form,
        import: presentation.import_form,
        rotate: presentation.rotate_form
      })

    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectSettingsPage.project_settings_page
        state={@presentation.status}
        project={@presentation.project}
        project_id={@presentation.project_id}
        item_count={@presentation.item_count}
        secrets={@streams.secrets}
        project_secrets={@presentation.secrets}
        secret_authority={@presentation.secret_authority}
        project_repositories={@presentation.repositories}
        form={@presentation.forms}
        organization_index_destination={~p"/organizations"}
        organization_destination={organization_destination(@presentation.project)}
        create_secret_event="create-secret"
        rotate_secret_event="rotate-secret"
        set_secret_enabled_event="set-secret-enabled"
        revoke_secret_event="revoke-secret"
        purge_secret_event="purge-secret"
        grant_secret_event="grant-secret"
        accept_import_event="accept-secret-import"
      />
    </Layouts.app>
    """
  end

  defp start_command(socket, command, attributes, sensitive?) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    {submitting, _effects} = ProjectSettingsState.reduce(state, :submitting)

    operation = fn ->
      if sensitive?,
        do: ProjectSettingsState.execute_sensitive(state, identity, command, attributes),
        else: ProjectSettingsState.execute(state, {:command, identity, command, attributes})
    end

    {:noreply, socket |> assign(:page_state, submitting) |> start_async(:command, operation)}
  end

  defp organization_destination(nil), do: "/organizations"
  defp organization_destination(project), do: "/organizations/#{project["organization_id"]}"
end
