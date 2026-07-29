defmodule HephaestusWeb.CommandClientTest do
  use ExUnit.Case, async: false

  alias HephaestusWeb.{CommandClient, Identity}

  @sentinel "HEPHAESTUS_UI_SENTINEL_7f47e9"

  setup do
    previous = Application.fetch_env!(:hephaestus_web, :internal_commands)

    on_exit(fn ->
      Application.put_env(:hephaestus_web, :internal_commands, previous)
    end)

    :ok
  end

  test "write-only values reach only the trusted command body and never the response" do
    test_process = self()

    plug = fn conn ->
      {:ok, body, conn} = Plug.Conn.read_body(conn)
      send(test_process, {:command_body, body, Plug.Conn.get_req_header(conn, "authorization")})
      Req.Test.json(conn, %{"secret_id" => Ecto.UUID.generate()})
    end

    Application.put_env(:hephaestus_web, :internal_commands,
      url: "http://internal.test/internal/v1/commands",
      token: "test-internal-command-token",
      plug: plug
    )

    identity = %Identity{
      user_id: Ecto.UUID.generate(),
      issuer: "https://issuer.test",
      subject: "reviewer",
      display_name: "Reviewer"
    }

    assert {:ok, response} =
             CommandClient.execute(identity, "create_secret", %{
               "owner" => %{"type" => "project", "id" => Ecto.UUID.generate()},
               "name" => "provider_token",
               "allowed_delivery_modes" => ["brokered"],
               "value" => @sentinel
             })

    assert_receive {:command_body, body, ["Bearer test-internal-command-token"]}
    assert body =~ @sentinel
    refute inspect(response) =~ @sentinel
    refute inspect(identity) =~ @sentinel
  end
end
