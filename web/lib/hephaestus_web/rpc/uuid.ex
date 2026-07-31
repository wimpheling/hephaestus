defmodule HephaestusWeb.RPC.UUID do
  @moduledoc false

  import Bitwise

  @spec generate() :: String.t()
  def generate do
    <<a::32, b::16, c::16, d::16, e::48>> = :crypto.strong_rand_bytes(16)
    version = (c &&& 0x0FFF) ||| 0x4000
    variant = (d &&& 0x3FFF) ||| 0x8000

    Enum.join(
      [hex(a, 8), hex(b, 4), hex(version, 4), hex(variant, 4), hex(e, 12)],
      "-"
    )
  end

  defp hex(value, width) do
    value
    |> Integer.to_string(16)
    |> String.pad_leading(width, "0")
  end
end
