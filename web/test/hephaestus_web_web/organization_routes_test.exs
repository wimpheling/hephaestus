defmodule HephaestusWebWeb.OrganizationRoutesTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{OrganizationWorkspaceLive, Router}

  @organization_id "018f689a-a81d-7c2e-943f-3a41f7981234"

  test "organization resources and forms have stable LiveView actions" do
    assert live_action("/organizations/#{@organization_id}") ==
             {OrganizationWorkspaceLive, :projects}

    assert live_action("/organizations/#{@organization_id}/secrets") ==
             {OrganizationWorkspaceLive, :secrets}

    assert live_action("/organizations/#{@organization_id}/secrets/new") ==
             {OrganizationWorkspaceLive, :new_secret}

    assert live_action("/organizations/#{@organization_id}/secret-grants/new") ==
             {OrganizationWorkspaceLive, :new_grant}
  end

  defp live_action(path) do
    Router
    |> Phoenix.Router.route_info("GET", path, "localhost")
    |> Map.fetch!(:phoenix_live_view)
    |> then(fn {module, action, _options, _session} -> {module, action} end)
  end
end
