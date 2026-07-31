defmodule HephaestusWebWeb.ConnCase do
  @moduledoc """
  This module defines the test case to be used by
  tests that require setting up a connection.

  Such tests rely on `Phoenix.ConnTest` and import the small set of
  conveniences needed to exercise the browser boundary. Backend state is
  supplied through RPC test doubles; Phoenix has no database sandbox or
  infrastructure client to configure here.
  """

  use ExUnit.CaseTemplate

  using do
    quote do
      # The default endpoint for testing
      @endpoint HephaestusWebWeb.Endpoint

      use HephaestusWebWeb, :verified_routes

      # Import conveniences for testing with connections
      import Plug.Conn
      import Phoenix.ConnTest
      import HephaestusWebWeb.ConnCase
    end
  end

  setup _tags, do: {:ok, conn: Phoenix.ConnTest.build_conn()}
end
