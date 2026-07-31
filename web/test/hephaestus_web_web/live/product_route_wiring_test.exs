defmodule HephaestusWebWeb.ProductRouteWiringTest do
  use ExUnit.Case, async: true

  @route_adapters [
    "organization_live.ex",
    "organization_workspace_live.ex",
    "organization_secrets_live.ex",
    "organization_new_secret_live.ex",
    "organization_new_grant_live.ex",
    "project_live.ex",
    "project_agents_live.ex",
    "project_runs_live.ex",
    "project_settings_live.ex"
  ]

  test "product LiveViews are callback adapters without backend clients" do
    for filename <- @route_adapters do
      source = File.read!(Path.join([File.cwd!(), "lib/hephaestus_web_web/live", filename]))

      refute source =~ "CommandClient"
      refute source =~ "Store."
      refute source =~ "RunNotifier"
      assert source =~ "assign(:page_state"
      assert source =~ "Page."
    end
  end

  test "sensitive callback parameters remain transient" do
    for filename <- [
          "organization_secrets_live.ex",
          "organization_new_secret_live.ex",
          "project_agents_live.ex",
          "project_settings_live.ex"
        ] do
      source = File.read!(Path.join([File.cwd!(), "lib/hephaestus_web_web/live", filename]))

      refute source =~ "assign(:secret"
      refute source =~ "assign(:password"
      refute source =~ "assign(:parameters"
    end
  end

  test "router keeps every product URL on its route-specific adapter" do
    router = File.read!(Path.join([File.cwd!(), "lib/hephaestus_web_web/router.ex"]))

    assert router =~ ~s(live "/organizations/:organization_id/secrets", OrganizationSecretsLive)
    assert router =~ "OrganizationNewSecretLive"
    assert router =~ "OrganizationNewGrantLive"
    assert router =~ ~s(live "/projects/:project_id/agents", ProjectAgentsLive)
    assert router =~ ~s(live "/projects/:project_id/runs", ProjectRunsLive)
    assert router =~ ~s(live "/projects/:project_id/settings", ProjectSettingsLive)
  end
end
