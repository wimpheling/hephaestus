defmodule HephaestusWebWeb.ProjectRoutesTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{
    AgentInstanceLive,
    ProjectAgentsLive,
    ProjectLive,
    ProjectRunsLive,
    ProjectSettingsLive,
    Router
  }

  @project_id "018f689a-a81d-7c2e-943f-3a41f7981234"
  @instance_id "018f689a-a81d-7c2e-943f-3a41f7984321"

  test "project resources resolve to exact LiveView actions" do
    assert live_action("/projects/#{@project_id}") == {ProjectLive, :repositories}
    assert live_action("/projects/#{@project_id}/repositories") == {ProjectLive, :repositories}
    assert live_action("/projects/#{@project_id}/agents") == {ProjectAgentsLive, :agents}
    assert live_action("/projects/#{@project_id}/runs") == {ProjectRunsLive, :runs}
    assert live_action("/projects/#{@project_id}/settings") == {ProjectSettingsLive, :settings}

    assert live_action("/projects/#{@project_id}/agents/#{@instance_id}") ==
             {AgentInstanceLive, :show}
  end

  defp live_action(path) do
    Router
    |> Phoenix.Router.route_info("GET", path, "localhost")
    |> Map.fetch!(:phoenix_live_view)
    |> then(fn {module, action, _options, _session} -> {module, action} end)
  end
end
