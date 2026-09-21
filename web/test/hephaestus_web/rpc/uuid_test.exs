defmodule HephaestusWeb.RPC.UUIDTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.RPC.UUID

  test "generates lowercase canonical UUID text" do
    values = Enum.map(1..500, fn _ -> UUID.generate() end)

    assert Enum.all?(values, fn value ->
             Regex.match?(
               ~r/\A[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/,
               value
             )
           end)
  end
end
