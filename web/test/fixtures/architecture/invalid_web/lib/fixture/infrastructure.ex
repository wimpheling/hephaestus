defmodule Fixture.Infrastructure do
  import Ecto.Query

  alias Postgrex

  def query(connection), do: Postgrex.query(connection, "SELECT secret FROM records", [])
  def subscribe, do: Gnat.start_link([])
  def database, do: System.get_env("DATABASE_URL")
end
