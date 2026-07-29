defmodule HephaestusWebWeb.AuthControllerTest do
  use HephaestusWebWeb.ConnCase

  @callback_params %{"code" => "stale-code", "state" => "stale-state"}
  @identity %{
    "display_name" => "Ada Reviewer",
    "issuer" => "http://127.0.0.1:5556",
    "subject" => "reviewer",
    "user_id" => "10000000-0000-4000-8000-000000000001"
  }

  test "an OIDC callback without a stored transaction returns to sign-in", %{conn: conn} do
    conn = get(conn, ~p"/auth/oidc/callback?#{@callback_params}")

    assert redirected_to(conn) == ~p"/"

    assert Phoenix.Flash.get(conn.assigns.flash, :error) ==
             "The sign-in request expired. Start sign-in again."

    refute get_session(conn, :oidc_session_params)
  end

  test "a stale callback preserves an existing authenticated session", %{conn: conn} do
    conn =
      conn
      |> init_test_session(%{"identity" => @identity})
      |> get(~p"/auth/oidc/callback?#{@callback_params}")

    assert redirected_to(conn) == ~p"/organizations"
    assert get_session(conn, :identity) == @identity
    refute get_session(conn, :oidc_session_params)
  end
end
