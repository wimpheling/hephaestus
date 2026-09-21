defmodule HephaestusWebWeb.AuthController do
  use HephaestusWebWeb, :controller

  alias Assent.Strategy.OIDC
  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.IdentityBootstrap

  def login(conn, _params) do
    config = Keyword.put(oidc_config(), :nonce, nonce())

    case OIDC.authorize_url(config) do
      {:ok, %{url: url, session_params: session_params}} ->
        conn
        |> put_session(:oidc_session_params, session_params)
        |> redirect(external: url)

      {:error, _reason} ->
        conn
        |> put_status(:service_unavailable)
        |> text("OIDC provider is unavailable.")
    end
  end

  def callback(conn, params) do
    case get_session(conn, :oidc_session_params) do
      %{state: state} = session_params when is_binary(state) ->
        complete_callback(conn, params, session_params)

      _missing_or_invalid_session ->
        reject_expired_callback(conn)
    end
  end

  defp complete_callback(conn, params, session_params) do
    config =
      Keyword.put(
        oidc_config(),
        :session_params,
        session_params
      )

    with {:ok, %{user: claims}} <- OIDC.callback(config, params),
         issuer <- Keyword.fetch!(config, :base_url),
         sid <- HephaestusWeb.RPC.UUID.generate(),
         conn <-
           complete_verified_callback(
             conn,
             issuer,
             claims,
             sid,
             &IdentityBootstrap.resolve/2,
             &IdentityBootstrap.create_session/3
           ) do
      conn
    else
      {:error, _reason} -> failure_redirect(conn)
    end
  end

  @doc false
  @spec complete_verified_callback(
          Plug.Conn.t(),
          String.t(),
          map(),
          String.t(),
          (String.t(), map() -> {:ok, Identity.t()} | {:error, term()}),
          (String.t(), map(), String.t() -> {:ok, map()} | {:error, term()})
        ) :: Plug.Conn.t()
  def complete_verified_callback(conn, issuer, claims, sid, resolve, create)
      when is_binary(issuer) and is_map(claims) and is_binary(sid) and
             is_function(resolve, 2) and is_function(create, 3) do
    case establish_identity(issuer, claims, sid, resolve, create) do
      {:ok, identity} ->
        conn
        |> delete_session(:oidc_session_params)
        |> put_session(:identity, Identity.to_session(identity))
        |> configure_session(renew: true)
        |> redirect(to: ~p"/organizations")

      {:error, _reason} ->
        failure_redirect(conn)
    end
  end

  defp establish_identity(issuer, claims, sid, resolve, create) do
    with {:ok, identity} <- resolve.(issuer, claims),
         {:ok, session} <- create.(issuer, claims, sid),
         {:ok, identity} <- Identity.attach_session(identity, sid, session) do
      {:ok, identity}
    end
  end

  defp failure_redirect(conn) do
    conn
    |> clear_session()
    |> put_flash(:error, "Sign-in failed. Start the sign-in flow again.")
    |> redirect(to: ~p"/")
  end

  defp reject_expired_callback(conn) do
    redirect_path =
      if conn.assigns.current_identity do
        ~p"/organizations"
      else
        ~p"/"
      end

    conn
    |> delete_session(:oidc_session_params)
    |> put_flash(:error, "The sign-in request expired. Start sign-in again.")
    |> redirect(to: redirect_path)
  end

  def logout(conn, _params) do
    case Map.get(conn.assigns, :current_identity) do
      %Identity{} = identity -> _ = IdentityBootstrap.revoke_session(identity)
      _missing_or_invalid_identity -> :ok
    end

    conn
    |> clear_session()
    |> configure_session(renew: true)
    |> redirect(to: ~p"/")
  end

  defp oidc_config do
    configuration = Application.fetch_env!(:hephaestus_web, :oidc)

    [
      base_url: Keyword.fetch!(configuration, :issuer),
      client_id: Keyword.fetch!(configuration, :client_id),
      client_secret: Keyword.fetch!(configuration, :client_secret),
      redirect_uri: Keyword.fetch!(configuration, :redirect_uri),
      authorization_params: [scope: "openid profile email"]
    ]
  end

  defp nonce do
    24
    |> :crypto.strong_rand_bytes()
    |> Base.url_encode64(padding: false)
  end
end
