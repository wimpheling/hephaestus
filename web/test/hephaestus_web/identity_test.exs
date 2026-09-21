defmodule HephaestusWeb.IdentityTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.Identity

  test "round-trips only a validated internal browser principal" do
    identity = %Identity{
      user_id: "01934f4c-9123-7a42-9f8e-21ad98f3c102",
      issuer: "https://issuer.example",
      subject: "stable-subject",
      display_name: "Ada",
      sid: "20000000-0000-4000-8000-000000000002",
      session_expires_at: 4_000_000_000
    }

    assert {:ok, ^identity} = identity |> Identity.to_session() |> Identity.from_session()
    assert :error = Identity.from_session(%{"user_id" => "../invalid"})
    assert :error = Identity.from_session(identity |> Identity.to_session() |> Map.delete("sid"))

    assert :error =
             Identity.from_session(
               identity
               |> Identity.to_session()
               |> Map.put("session_expires_at", 1)
             )

    refute inspect(identity) =~ identity.sid
  end

  test "accepts only a matching non-expired browser-session response" do
    identity = %Identity{
      user_id: "01934f4c-9123-7a42-9f8e-21ad98f3c102",
      issuer: "https://issuer.example",
      subject: "stable-subject",
      display_name: "Ada",
      sid: "20000000-0000-4000-8000-000000000002",
      session_expires_at: 4_000_000_000
    }

    response = %{
      user_id: identity.user_id,
      session_id: "30000000-0000-4000-8000-000000000003",
      expires_at: DateTime.add(DateTime.utc_now(), 3600, :second),
      receipt: %{
        "committed_cursor" => "cursor-1",
        "aggregate_version" => 1,
        "event_id" => "60000000-0000-4000-8000-000000000006"
      }
    }

    assert {:ok, %Identity{sid: sid}} =
             Identity.attach_session(
               identity,
               "40000000-0000-4000-8000-000000000004",
               response
             )

    assert sid == "40000000-0000-4000-8000-000000000004"

    assert {:error, :invalid_response} =
             Identity.attach_session(
               identity,
               identity.sid,
               %{response | user_id: "50000000-0000-4000-8000-000000000005"}
             )

    assert {:error, :invalid_response} =
             Identity.attach_session(
               identity,
               identity.sid,
               %{response | expires_at: DateTime.utc_now()}
             )

    assert {:error, :invalid_response} =
             Identity.attach_session(identity, identity.sid, %{response | receipt: nil})

    assert {:error, :invalid_response} =
             Identity.attach_session(identity, identity.sid, %{response | receipt: %{"id" => "x"}})
  end
end
