defmodule HephaestusWebWeb.PersonalAccessTokensLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.PersonalAccessTokensPage
  alias HephaestusWebWeb.PersonalAccessTokensState

  @stream_mode :none

  @impl true
  def mount(_params, _session, socket) do
    _stream_mode = @stream_mode
    state = PersonalAccessTokensState.new(%{})
    socket = assign(socket, page_state: state, page_title: "Git credentials")

    if connected?(socket) do
      {state, effects} = PersonalAccessTokensState.reduce(state, :load)
      {:ok, socket |> assign(:page_state, state) |> apply_effects(effects)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_event("create-personal-access-token", %{"token" => attributes}, socket),
    do: start_command(socket, {:create, attributes})

  def handle_event(
        "rotate-personal-access-token",
        %{"token_id" => token_id, "rotation" => attributes},
        socket
      ),
      do: start_command(socket, {:rotate, token_id, attributes})

  def handle_event("revoke-personal-access-token", %{"token_id" => token_id}, socket),
    do: start_command(socket, {:revoke, token_id})

  @impl true
  def handle_async(:load, {:ok, event}, socket), do: reduce_event(socket, event)
  def handle_async(:load, {:exit, reason}, socket), do: reduce_event(socket, {:failed, reason})
  def handle_async(:command, {:ok, event}, socket), do: reduce_event(socket, event)
  def handle_async(:command, {:exit, reason}, socket), do: reduce_event(socket, {:failed, reason})

  @impl true
  def render(assigns) do
    presentation = PersonalAccessTokensState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <PersonalAccessTokensPage.personal_access_tokens_page
        state={@presentation.status}
        tokens={@presentation.tokens}
        item_count={@presentation.item_count}
        form={@presentation.form}
        error={@presentation.error}
        create_event="create-personal-access-token"
        rotate_event="rotate-personal-access-token"
        revoke_event="revoke-personal-access-token"
      />
    </Layouts.app>
    """
  end

  defp start_command(socket, command) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    {state, _effects} = PersonalAccessTokensState.reduce(state, :submitting)

    operation = fn ->
      case command do
        {:create, attributes} ->
          PersonalAccessTokensState.execute(state, {:create, identity, attributes})

        {:rotate, token_id, attributes} ->
          PersonalAccessTokensState.execute(state, {:rotate, identity, token_id, attributes})

        {:revoke, token_id} ->
          PersonalAccessTokensState.execute(state, {:revoke, identity, token_id})
      end
    end

    {:noreply, socket |> assign(:page_state, state) |> start_async(:command, operation)}
  end

  defp reduce_event(socket, event) do
    {state, effects} = PersonalAccessTokensState.reduce(socket.assigns.page_state, event)
    {:noreply, socket |> assign(:page_state, state) |> apply_effects(effects)}
  end

  defp apply_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:load, generation}, socket ->
        state = socket.assigns.page_state
        identity = socket.assigns.current_identity

        start_async(socket, :load, fn ->
          PersonalAccessTokensState.execute(state, {:load, identity, generation})
        end)

      {:reveal, value}, socket ->
        push_event(socket, "personal-access-token-issued", %{value: value})

      {:flash, kind, message}, socket ->
        put_flash(socket, kind, message)
    end)
  end
end
