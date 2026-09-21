defmodule HephaestusWebWeb.AuthControllerTest do
  use HephaestusWebWeb.ConnCase

  alias HephaestusWebWeb.AuthController
  alias HephaestusWeb.Identity

  @callback_params %{"code" => "stale-code", "state" => "stale-state"}
  @identity %{
    "display_name" => "Ada Reviewer",
    "issuer" => "http://127.0.0.1:5556",
    "subject" => "reviewer",
    "user_id" => "10000000-0000-4000-8000-000000000001",
    "sid" => "20000000-0000-4000-8000-000000000002",
    "session_expires_at" => 4_000_000_000
  }

  @claims %{"sub" => "reviewer", "name" => "Ada Reviewer"}
  @sid "30000000-0000-4000-8000-000000000003"

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

  test "logout clears the local cookie when remote revocation is unavailable", %{conn: conn} do
    previous_rpc = Application.fetch_env!(:hephaestus_web, :rpc)

    Application.put_env(
      :hephaestus_web,
      :rpc,
      Keyword.put(previous_rpc, :endpoint, "127.0.0.1:1")
    )

    HephaestusWeb.RPC.Channel.reset()

    on_exit(fn ->
      Application.put_env(:hephaestus_web, :rpc, previous_rpc)
      HephaestusWeb.RPC.Channel.reset()
    end)

    conn =
      conn
      |> init_test_session(%{"identity" => @identity})
      |> delete(~p"/logout")

    assert redirected_to(conn) == ~p"/"
    assert get_session(conn, :identity) == nil
  end

  test "verified callback completion stores the returned session and redirects", %{conn: conn} do
    identity = provisional_identity()

    conn =
      conn
      |> init_test_session(%{
        "identity" => @identity,
        "oidc_session_params" => %{state: "callback-state"}
      })
      |> fetch_flash()
      |> AuthController.complete_verified_callback(
        "https://issuer.example",
        @claims,
        @sid,
        fn _issuer, _claims -> {:ok, identity} end,
        fn _issuer, _claims, @sid -> {:ok, session_response(identity.user_id, @sid)} end
      )

    assert redirected_to(conn) == ~p"/organizations"
    assert get_session(conn, :oidc_session_params) == nil
    assert get_session(conn, :identity)["sid"] == @sid
    assert get_session(conn, :identity)["session_expires_at"] > System.system_time(:second)
  end

  test "verified callback mismatch clears a preexisting identity and transaction", %{conn: conn} do
    conn =
      conn
      |> init_test_session(%{
        "identity" => @identity,
        "oidc_session_params" => %{state: "callback-state"}
      })
      |> fetch_flash()
      |> AuthController.complete_verified_callback(
        "https://issuer.example",
        @claims,
        @sid,
        fn _issuer, _claims -> {:ok, provisional_identity()} end,
        fn _issuer, _claims, _sid ->
          {:ok, session_response("40000000-0000-4000-8000-000000000004", @sid)}
        end
      )

    assert redirected_to(conn) == ~p"/"
    assert get_session(conn, :identity) == nil
    assert get_session(conn, :oidc_session_params) == nil
  end

  test "verified callback create failure clears a preexisting identity", %{conn: conn} do
    conn =
      conn
      |> init_test_session(%{
        "identity" => @identity,
        "oidc_session_params" => %{state: "callback-state"}
      })
      |> fetch_flash()
      |> AuthController.complete_verified_callback(
        "https://issuer.example",
        @claims,
        @sid,
        fn _issuer, _claims -> {:ok, provisional_identity()} end,
        fn _issuer, _claims, _sid -> {:error, :unavailable} end
      )

    assert redirected_to(conn) == ~p"/"
    assert get_session(conn, :identity) == nil
    assert get_session(conn, :oidc_session_params) == nil
  end

  defp provisional_identity do
    @identity
    |> Identity.from_session()
    |> elem(1)
    |> Map.merge(%{sid: nil, session_expires_at: nil})
  end

  defp session_response(user_id, _sid) do
    %{
      user_id: user_id,
      session_id: "50000000-0000-4000-8000-000000000005",
      expires_at: DateTime.add(DateTime.utc_now(), 3600, :second),
      receipt: %{
        "committed_cursor" => "cursor-1",
        "aggregate_version" => 1,
        "event_id" => "60000000-0000-4000-8000-000000000006"
      }
    }
  end
end
