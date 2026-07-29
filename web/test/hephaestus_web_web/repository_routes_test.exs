defmodule HephaestusWebWeb.RepositoryRoutesTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{ReleaseLive, RepositoryLive, Router}

  @repository_id "018f689a-a81d-7c2e-943f-3a41f7981234"
  @release_id "018f689a-a81d-7c2e-943f-3a41f7985678"

  test "repository resources resolve to explicit LiveView actions" do
    assert live_action("/repositories/#{@repository_id}") == {RepositoryLive, :files}
    assert live_action("/repositories/#{@repository_id}/files") == {RepositoryLive, :files}
    assert live_action("/repositories/#{@repository_id}/commits") == {RepositoryLive, :commits}
    assert live_action("/repositories/#{@repository_id}/branches") == {RepositoryLive, :branches}
    assert live_action("/repositories/#{@repository_id}/releases") == {RepositoryLive, :releases}

    assert live_action("/repositories/#{@repository_id}/releases/#{@release_id}") ==
             {ReleaseLive, :show}

    assert live_action("/repositories/#{@repository_id}/agents") == {RepositoryLive, :agents}
  end

  test "file paths remain route data rather than a query-time revision expression" do
    route =
      Phoenix.Router.route_info(
        Router,
        "GET",
        "/repositories/#{@repository_id}/files/lib/agent.ex",
        "localhost"
      )

    assert route.phoenix_live_view
           |> then(fn {module, action, _options, _session} ->
             {module, action}
           end) == {RepositoryLive, :files}

    assert route.path_params == %{
             "repository_id" => @repository_id,
             "path" => ["lib", "agent.ex"]
           }
  end

  defp live_action(path) do
    Router
    |> Phoenix.Router.route_info("GET", path, "localhost")
    |> Map.fetch!(:phoenix_live_view)
    |> then(fn {module, action, _options, _session} -> {module, action} end)
  end
end
