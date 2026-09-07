defmodule HephaestusWebWeb.ProjectGatewayLive do
  @moduledoc "Route adapter for an authorized gateway revision and lifecycle view."

  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectGatewayPage
  alias HephaestusWebWeb.{PageStream, ProjectGatewayState}

  @stream_mode :page_scoped

  @impl true
  def mount(%{"project_id" => project_id, "gateway_id" => gateway_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectGatewayState.new(%{project_id: project_id, gateway_id: gateway_id})

    socket =
      socket
      |> assign(:project_id, project_id)
      |> assign(:gateway_id, gateway_id)
      |> assign(:page_state, state)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:transition_task, nil)
      |> assign(:page_title, "Gateway")

    if connected?(socket) do
      {:ok,
       socket
       |> PageStream.start_watch(ProjectGatewayState)
       |> PageStream.start_snapshot(ProjectGatewayState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_event("lifecycle", %{"next" => next}, socket) do
    {state, [{:lifecycle, next}]} =
      ProjectGatewayState.reduce(socket.assigns.page_state, {:lifecycle, next})

    identity = socket.assigns.current_identity

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> start_async(:transition, fn ->
       ProjectGatewayState.execute(state, {:lifecycle, identity, next})
     end)}
  end

  @impl true
  def handle_event("configure", %{"configure" => attributes}, socket) do
    {state, _effects} =
      ProjectGatewayState.reduce(socket.assigns.page_state, {:configure, attributes})

    identity = socket.assigns.current_identity

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> start_async(:configure, fn ->
       ProjectGatewayState.execute(state, {:configure, identity, attributes})
     end)}
  end

  @impl true
  def handle_event("bind-mailbox", %{"gateway_binding" => attributes}, socket) do
    {state, _effects} =
      ProjectGatewayState.reduce(socket.assigns.page_state, {:bind, attributes})

    identity = socket.assigns.current_identity

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> start_async(:bind_mailbox, fn ->
       ProjectGatewayState.execute(state, {:bind, identity, attributes})
     end)}
  end

  @impl true
  def handle_async(:transition, {:ok, {:ok, response}}, socket) do
    {state, effects} =
      ProjectGatewayState.reduce(socket.assigns.page_state, {:transitioned, response})

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  def handle_async(:transition, _result, socket) do
    {state, effects} = ProjectGatewayState.reduce(socket.assigns.page_state, :lifecycle_failed)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> put_flash(:error, state.error)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  @impl true
  def handle_async(:configure, {:ok, {:configured, generation, response}}, socket) do
    {state, effects} =
      ProjectGatewayState.reduce(socket.assigns.page_state, {:configured, generation, response})

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  def handle_async(:configure, {:ok, {:failed, _reason}}, socket) do
    {state, effects} = ProjectGatewayState.reduce(socket.assigns.page_state, :configure_failed)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> put_flash(:error, state.error)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  def handle_async(:configure, {:exit, _reason}, socket) do
    {state, effects} = ProjectGatewayState.reduce(socket.assigns.page_state, :configure_failed)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> put_flash(:error, state.error)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  @impl true
  def handle_async(:bind_mailbox, {:ok, {:bound, generation, response}}, socket) do
    {state, effects} =
      ProjectGatewayState.reduce(socket.assigns.page_state, {:bound, generation, response})

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> put_flash(:info, "Mailbox binding created.")
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  def handle_async(:bind_mailbox, {:ok, {:failed, _reason}}, socket) do
    {state, effects} = ProjectGatewayState.reduce(socket.assigns.page_state, :bind_failed)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> put_flash(:error, state.error)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  def handle_async(:bind_mailbox, {:exit, _reason}, socket) do
    {state, effects} = ProjectGatewayState.reduce(socket.assigns.page_state, :bind_failed)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> put_flash(:error, state.error)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, ProjectGatewayState, response)
    {:noreply, PageStream.apply_effects(socket, ProjectGatewayState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, ProjectGatewayState, result)
    {:noreply, PageStream.apply_effects(socket, ProjectGatewayState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectGatewayState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectGatewayState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
    PageStream.cancel(socket.assigns[:watch_task])
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  @impl true
  def render(assigns) do
    presentation = ProjectGatewayState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectGatewayPage.project_gateway_page
        state={@presentation.status}
        gateway={@presentation.gateway}
        ingress={@presentation.ingress}
        instances={@presentation.instances}
        bindings={@presentation.bindings}
        binding_slots={@presentation.binding_slots}
        binding_form={@presentation.binding_form}
        release_catalog={@presentation.release_catalog}
        secret_authority={@presentation.secret_authority}
        secrets={@presentation.secrets}
        configure_form={@presentation.configure_form}
        gateways_destination={~p"/projects/#{@project_id}/gateways"}
        lifecycle_event="lifecycle"
        lifecycle_actions={@presentation.lifecycle_actions}
        configure_event="configure"
        bind_event="bind-mailbox"
      />
    </Layouts.app>
    """
  end
end
