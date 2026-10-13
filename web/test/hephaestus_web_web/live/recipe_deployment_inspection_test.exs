defmodule HephaestusWebWeb.RecipeDeploymentInspectionTest do
  @moduledoc "Controlled transport/presentation models, not PostgreSQL provenance tests."
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias Hephaestus.Common.V1.OpaqueId

  alias Hephaestus.Project.V1.{
    GetRecipeDeploymentResponse,
    RecipeDeployment,
    RecipeDeploymentResource
  }

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.Error
  alias HephaestusWebWeb.ProjectState
  alias HephaestusWebWeb.DesignSystem.Pages.ProjectPage

  @project "10000000-0000-4000-8000-000000000001"
  @deployment "20000000-0000-4000-8000-000000000002"

  test "current identity calls the generated query with exact original ids" do
    owner = self()

    identity = %Identity{
      user_id: "90000000-0000-4000-8000-000000000009",
      issuer: "https://issuer.example",
      subject: "subject",
      display_name: "User",
      sid: "a0000000-0000-4000-8000-00000000000a"
    }

    state = ProjectState.new(%{project_id: @project})
    {state, [effect]} = ProjectState.reduce(state, {:inspect_recipe, @deployment})

    stub = fn _channel, request, options ->
      send(owner, {:query, request, options})
      {:ok, %GetRecipeDeploymentResponse{deployment: wire()}}
    end

    event =
      ProjectState.execute_recipe(state, effect, identity,
        stub_call: stub,
        channel_provider: fn -> {:ok, %GRPC.Channel{}} end
      )

    assert_receive {:query, request, options}
    assert request.project_id.value == @project
    assert request.deployment_id.value == @deployment
    assert is_map(options[:metadata])
    {state, []} = ProjectState.reduce(state, event)
    assert state.data.recipe_inspection.status == :ready
    assert state.data.recipe_inspection.deployment["activation"] == "unobserved"

    assert state.data.recipe_inspection.deployment["resources"] |> hd() |> Map.fetch!("removal") ==
             "retained"
  end

  test "a newer exact-id query rejects old results and clears old data" do
    state = ProjectState.new(%{project_id: @project})
    {first, [first_effect]} = ProjectState.reduce(state, {:inspect_recipe, @deployment})

    {second, [_effect]} =
      ProjectState.reduce(first, {:inspect_recipe, "30000000-0000-4000-8000-000000000003"})

    {:inspect_recipe, generation, _id} = first_effect

    assert {second, []} ==
             ProjectState.reduce(second, {:recipe_inspected, generation, {:ok, %{}}})

    assert second.data.recipe_inspection.deployment == nil
    {loaded, []} = ProjectState.reduce(second, {:loaded, second.stream_generation, %{}, []})
    assert loaded.data.recipe_inspection == second.data.recipe_inspection
    {reconnecting, []} = ProjectState.reduce(loaded, :reconnecting)
    {reloaded, []} = ProjectState.reduce(reconnecting, {:loaded, -1, %{name: "new"}, []})
    assert reloaded.data.project == %{name: "new"}
    assert reloaded.data.recipe_inspection == second.data.recipe_inspection
  end

  test "project access revocation clears inspection and rejects a pending response" do
    state = ProjectState.new(%{project_id: @project})

    {state, [{:inspect_recipe, generation, _id}]} =
      ProjectState.reduce(state, {:inspect_recipe, @deployment})

    for event <- [
          {:access_revoked, :denied},
          {:watch, %{item: {:access_revoked, %{}}}}
        ] do
      {revoked, [{:navigate, :organizations}]} = ProjectState.reduce(state, event)
      assert revoked.data.recipe_inspection.status == :denied
      assert revoked.data.recipe_inspection.deployment == nil
      assert revoked.data.recipe_inspection.generation == generation + 1
      assert ProjectState.present(revoked).recipe_inspection == revoked.data.recipe_inspection

      assert {revoked, []} ==
               ProjectState.reduce(revoked, {:recipe_inspected, generation, {:ok, %{}}})
    end
  end

  test "invalid, denied, unavailable and contradictory originals are distinct" do
    state = ProjectState.new(%{project_id: @project})
    {invalid, []} = ProjectState.reduce(state, {:inspect_recipe, "not-an-id"})
    assert invalid.data.recipe_inspection.status == :invalid

    {loading, [{:inspect_recipe, generation, _id}]} =
      ProjectState.reduce(state, {:inspect_recipe, @deployment})

    for {kind, expected} <- [
          permission_denied: :denied,
          unauthenticated: :denied,
          unavailable: :unavailable,
          precondition: :error
        ] do
      {result, []} =
        ProjectState.reduce(loading, {:recipe_inspected, generation, {:error, Error.local(kind)}})

      assert result.data.recipe_inspection.status == expected
      assert result.data.recipe_inspection.deployment == nil

      if kind in [:permission_denied, :unauthenticated] do
        assert {result, []} ==
                 ProjectState.reduce(result, {:recipe_inspected, generation, {:ok, %{}}})
      end
    end
  end

  test "held and retained progress is rendered without running or deletion inference" do
    state = %{
      status: :ready,
      deployment_id: @deployment,
      error: nil,
      generation: 1,
      deployment: %{
        "id" => @deployment,
        "key" => "sqlite",
        "version" => 9,
        "lifecycle" => "recovery_required",
        "resources" => [
          %{
            "name" => "data",
            "kind" => "volume",
            "ownership" => "owned",
            "resource_id" => "volume-id",
            "revision_id" => nil,
            "install" => "recovery_required",
            "removal" => "retained",
            "version" => 7,
            "active_attempt_id" => "attempt-id",
            "diagnostic" => "provider_outcome_unknown",
            "recovery_required" => true
          }
        ]
      }
    }

    html = render_inspection(state)
    assert html =~ "Held for reconciliation"
    assert html =~ "retained"
    assert html =~ "Activation is unobserved"
    assert html =~ "does not confirm a running workload"
    assert html =~ "inspect-recipe-deployment-form"
    refute html =~ "Install recipe"
    refute html =~ "Invoke"
    refute html =~ "Remove deployment"
  end

  test "unavailable rendering does not invent an absent deployment" do
    state = %{
      status: :unavailable,
      deployment_id: @deployment,
      deployment: nil,
      generation: 1,
      error: "Deployment inspection is unavailable. No absence or completion is confirmed."
    }

    html = render_inspection(state)
    assert html =~ "No absence or completion is confirmed"
    refute html =~ "No deployments"
  end

  test "unknown wire progress and missing deployment deny instead of defaulting" do
    identity = %Identity{
      user_id: "90000000-0000-4000-8000-000000000009",
      issuer: "https://issuer.example",
      subject: "subject",
      display_name: "User",
      sid: "a0000000-0000-4000-8000-00000000000a"
    }

    for value <- [nil, %{wire() | activation: 99}] do
      assert {:error, %Error{kind: :precondition}} =
               HephaestusWeb.RPC.Client.get_recipe_deployment(identity, @project, @deployment,
                 stub_call: fn _channel, _request, _options ->
                   {:ok, %GetRecipeDeploymentResponse{deployment: value}}
                 end,
                 channel_provider: fn -> {:ok, %GRPC.Channel{}} end
               )
    end
  end

  defp render_inspection(state) do
    render_component(&ProjectPage.project_page/1,
      state: :ready,
      project: %{
        "id" => @project,
        "name" => "Project",
        "organization_id" => "organization",
        "organization_name" => "Organization"
      },
      project_id: @project,
      repositories: [],
      recipe_inspection: state,
      recipe_form:
        Phoenix.Component.to_form(%{"deployment_id" => state.deployment_id}, as: :recipe),
      organization_index_destination: "/organizations",
      organization_destination: "/organizations/organization",
      repository_destination: &"/repositories/#{&1}"
    )
  end

  defp wire do
    %RecipeDeployment{
      id: %OpaqueId{value: @deployment},
      project_id: %OpaqueId{value: @project},
      key: "sqlite",
      version: 9,
      lifecycle: :RECIPE_DEPLOYMENT_LIFECYCLE_RECOVERY_REQUIRED,
      activation: :RECIPE_ACTIVATION_OBSERVATION_UNOBSERVED,
      resources: [
        %RecipeDeploymentResource{
          name: "data",
          kind: :RECIPE_RESOURCE_KIND_VOLUME,
          resource_id: %OpaqueId{value: "40000000-0000-4000-8000-000000000004"},
          ownership: :RECIPE_RESOURCE_OWNERSHIP_OWNED,
          install: :RECIPE_INSTALL_PROGRESS_RECOVERY_REQUIRED,
          removal: :RECIPE_REMOVAL_PROGRESS_RETAINED,
          version: 7,
          recovery_required: true,
          diagnostic: :RECIPE_PROGRESS_DIAGNOSTIC_PROVIDER_OUTCOME_UNKNOWN
        }
      ]
    }
  end
end
