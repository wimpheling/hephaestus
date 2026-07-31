defmodule HephaestusWebWeb.InfrastructureIsolationTest do
  use HephaestusWebWeb.ConnCase, async: false

  @identity %{
    "display_name" => "RPC Reviewer",
    "issuer" => "https://issuer.example",
    "subject" => "reviewer",
    "user_id" => "10000000-0000-4000-8000-000000000001"
  }

  @removed_children [
    HephaestusWeb.Repo,
    HephaestusWeb.Store,
    HephaestusWeb.RunNotifier,
    HephaestusWeb.RepositoryBrowser,
    HephaestusWeb.ArtifactStore
  ]

  test "the running Phoenix application owns only browser and RPC processes", %{conn: conn} do
    assert Process.whereis(HephaestusWeb.Supervisor)
    assert Process.whereis(HephaestusWeb.RPC.Channel)
    assert Process.whereis(HephaestusWeb.PageTaskSupervisor)

    child_ids =
      HephaestusWeb.Supervisor
      |> Supervisor.which_children()
      |> Enum.map(&elem(&1, 0))

    assert Enum.all?(@removed_children, &(&1 not in child_ids))

    applications = Application.spec(:hephaestus_web, :applications)
    refute :ecto in applications
    refute :postgrex in applications

    assert conn |> get(~p"/") |> html_response(200) =~ "Sign in with OIDC"
  end

  test "an authenticated page renders a bounded fallback without local infrastructure", %{
    conn: conn
  } do
    response =
      conn
      |> init_test_session(%{"identity" => @identity})
      |> get(~p"/organizations")
      |> html_response(200)

    assert response =~ "Organizations"
    assert response =~ "The organization perimeter is not ready."
    refute response =~ "connection_refused"
    assert Process.alive?(Process.whereis(HephaestusWeb.Supervisor))
  end
end
