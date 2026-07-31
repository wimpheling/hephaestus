defmodule Fixture.Pages.DashboardState do
  alias Fixture.Generated.DashboardClient

  def load, do: DashboardClient.list_dashboards()
end
