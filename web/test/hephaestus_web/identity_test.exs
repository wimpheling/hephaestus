defmodule HephaestusWeb.IdentityTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.Identity

  test "round-trips only a validated internal browser principal" do
    identity = %Identity{
      user_id: "01934f4c-9123-7a42-9f8e-21ad98f3c102",
      issuer: "https://issuer.example",
      subject: "stable-subject",
      display_name: "Ada"
    }

    assert {:ok, ^identity} = identity |> Identity.to_session() |> Identity.from_session()
    assert :error = Identity.from_session(%{"user_id" => "../invalid"})
  end
end
