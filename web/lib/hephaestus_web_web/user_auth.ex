defmodule HephaestusWebWeb.UserAuth do
  @moduledoc false

  import Phoenix.Controller
  import Plug.Conn
  alias HephaestusWeb.Identity

  def init(action), do: action

  def call(conn, :fetch_current_identity), do: fetch_current_identity(conn, [])
  def call(conn, :require_authenticated), do: require_authenticated(conn, [])

  def fetch_current_identity(conn, _options) do
    identity =
      case get_session(conn, :identity) do
        nil -> nil
        session -> with {:ok, identity} <- Identity.from_session(session), do: identity
      end

    assign(conn, :current_identity, identity)
  end

  def require_authenticated(%{assigns: %{current_identity: %Identity{}}} = conn, _options),
    do: conn

  def require_authenticated(conn, _options) do
    conn
    |> put_flash(:error, "Sign in to review agent runs.")
    |> redirect(to: "/login")
    |> halt()
  end

  def on_mount(:require_authenticated, _params, session, socket) do
    with identity_session when not is_nil(identity_session) <- session["identity"],
         {:ok, identity} <- Identity.from_session(identity_session) do
      {:cont, Phoenix.Component.assign(socket, :current_identity, identity)}
    else
      _ ->
        {:halt,
         socket
         |> Phoenix.LiveView.put_flash(:error, "Your session has expired.")
         |> Phoenix.LiveView.redirect(to: "/login")}
    end
  end
end
