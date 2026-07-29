defmodule HephaestusWebWeb.AuthController do
  use HephaestusWebWeb, :controller

  alias Assent.Strategy.OIDC
  alias HephaestusWeb.{Identity, IdentityMapper}

  def login(conn, _params) do
    config = Keyword.put(oidc_config(), :nonce, nonce())

    case OIDC.authorize_url(config) do
      {:ok, %{url: url, session_params: session_params}} ->
        conn
        |> put_session(:oidc_session_params, session_params)
        |> redirect(external: url)

      {:error, reason} ->
        conn
        |> put_status(:service_unavailable)
        |> text("OIDC provider is unavailable: #{inspect(reason)}")
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
         {:ok, identity} <- IdentityMapper.map_verified(issuer, claims) do
      conn
      |> delete_session(:oidc_session_params)
      |> put_session(:identity, Identity.to_session(identity))
      |> configure_session(renew: true)
      |> redirect(to: ~p"/organizations")
    else
      {:error, reason} ->
        conn
        |> clear_session()
        |> put_flash(:error, "Sign-in failed: #{inspect(reason)}")
        |> redirect(to: ~p"/")
    end
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
