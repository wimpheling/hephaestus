defmodule HephaestusWeb.IdentityTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.Identity

  test "round-trips only a validated internal browser principal" do
    identity = %Identity{
      user_id: Ecto.UUID.generate(),
      issuer: "https://issuer.example",
      subject: "stable-subject",
      display_name: "Ada"
    }

    assert {:ok, ^identity} = identity |> Identity.to_session() |> Identity.from_session()
    assert :error = Identity.from_session(%{"user_id" => "../invalid"})
  end
end
