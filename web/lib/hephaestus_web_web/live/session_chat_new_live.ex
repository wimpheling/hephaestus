defmodule HephaestusWebWeb.SessionChatNewLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.SessionChatNewPage
  alias HephaestusWebWeb.SessionChatNewState

  @stream_mode :none

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = SessionChatNewState.new(project_id)
    {state, _effects} = SessionChatNewState.reduce(state, :load)

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:page_title, "New session chat")

    if connected?(socket) do
      identity = socket.assigns.current_identity

      {:ok,
       start_async(socket, :load, fn -> SessionChatNewState.execute(state, {:load, identity}) end)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_event("create-session-chat", %{"session_chat" => attributes}, socket) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    {submitting, _effects} = SessionChatNewState.reduce(state, :submitting)
    {submitting, _effects} = SessionChatNewState.reduce(submitting, {:form, attributes})

    {:noreply,
     socket
     |> assign(:page_state, submitting)
     |> start_async(:command, fn ->
       SessionChatNewState.execute(state, {:create, identity, attributes})
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
    presentation = SessionChatNewState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      <SessionChatNewPage.session_chat_new
        state={@presentation.state}
        project={@presentation.project}
        project_id={@presentation.project_id}
        release_catalog={@presentation.release_catalog}
        secret_imports={@presentation.secret_imports}
        error={@presentation.error}
        progress={@presentation.progress}
        attempt_id={@presentation.attempt_id}
        form={to_form(@presentation.form, as: :session_chat)}
        create_event="create-session-chat"
      />
    </Layouts.app>
    """
  end

  defp reduce_event(socket, event) do
    {state, effects} = SessionChatNewState.reduce(socket.assigns.page_state, event)
    apply_effects(assign(socket, :page_state, state), effects)
  end

  defp apply_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:flash, kind, message}, socket ->
        put_flash(socket, kind, message)

      {:navigate, destination}, socket ->
        push_navigate(socket, to: destination)

      {:launch, url}, socket ->
        push_event(socket, "ui-browser-launch", %{"url" => url, "mode" => "full_page"})

      _effect, socket ->
        socket
    end)
  end
end
