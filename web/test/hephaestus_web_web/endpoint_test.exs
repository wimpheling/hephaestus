defmodule HephaestusWebWeb.EndpointTest do
  use ExUnit.Case, async: true

  import Plug.Conn

  @secret_key_base "test-secret-key-base-with-at-least-sixty-four-characters-1234567890"
  @identity %{"user_id" => "10000000-0000-4000-8000-000000000001"}

  test "the endpoint uses the configured development profile in test" do
    assert HephaestusWebWeb.Endpoint.session_options() ==
             HephaestusWebWeb.SessionOptions.for_profile(:development)
  end

  test "production session emits a host-only secure cookie" do
    options = HephaestusWebWeb.SessionOptions.for_profile(:production)
    response = write_session(options)
    [header] = get_resp_header(response, "set-cookie")

    assert header =~ "__Host-hephaestus_web_key="
    assert header =~ "; path=/"
    assert header =~ "; secure"
    assert header =~ "; HttpOnly"
    assert header =~ "; SameSite=Lax"
    refute header =~ ~r/(?:^|;\s*)domain=/i
  end

  test "development and test profiles remain usable over HTTP" do
    for profile <- [:development, :test] do
      options = HephaestusWebWeb.SessionOptions.for_profile(profile)
      response = write_session(options)
      [header] = get_resp_header(response, "set-cookie")

      assert header =~ "_hephaestus_web_key="
      refute header =~ "__Host-hephaestus_web_key="
      refute header =~ "; secure"
      assert header =~ "; path=/"
    end
  end

  test "a legacy cookie is ignored by the production profile" do
    legacy_cookie =
      :development
      |> HephaestusWebWeb.SessionOptions.for_profile()
      |> write_session()
      |> get_resp_header("set-cookie")
      |> List.first()
      |> String.split(";", parts: 2)
      |> List.first()

    legacy_conn = read_session(legacy_cookie, :development)
    assert get_session(legacy_conn, :identity) == @identity

    production_conn = read_session(legacy_cookie, :production)
    assert get_session(production_conn, :identity) == nil
  end

  defp write_session(options) do
    Plug.Test.conn(:get, "/")
    |> Map.put(:secret_key_base, @secret_key_base)
    |> Plug.Session.call(Plug.Session.init(options))
    |> fetch_session()
    |> put_session(:identity, @identity)
    |> send_resp(200, "ok")
  end

  defp read_session(cookie, profile) do
    Plug.Test.conn(:get, "/")
    |> Map.put(:secret_key_base, @secret_key_base)
    |> put_req_header("cookie", cookie)
    |> Plug.Session.call(Plug.Session.init(HephaestusWebWeb.SessionOptions.for_profile(profile)))
    |> fetch_session()
  end
end
