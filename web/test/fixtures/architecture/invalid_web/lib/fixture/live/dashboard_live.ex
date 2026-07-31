defmodule Fixture.DashboardLive do
  alias Fixture.Generated.DashboardClient

  def mount, do: DashboardClient.list_dashboards()
end
