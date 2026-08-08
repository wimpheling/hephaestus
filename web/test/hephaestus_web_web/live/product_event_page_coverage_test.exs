defmodule HephaestusWebWeb.ProductEventPageCoverageTest do
  use ExUnit.Case, async: true

  alias Hephaestus.Event.V1.ProductEvent
  alias HephaestusWebWeb.ProductEventReducer

  alias HephaestusWebWeb.{
    AgentInstanceState,
    OrganizationState,
    OrganizationWorkspaceState,
    ProjectRunsState,
    ProjectState,
    ReleaseState,
    RunState
  }

  @coverage [
    {OrganizationState, [:identity_profile_changed, :identity_organizations_changed]},
    {OrganizationWorkspaceState, [:organization_changed, :project_changed, :repository_changed]},
    {ProjectState, [:project_changed, :repository_changed]},
    {ProjectRunsState, [:project_changed, :run_changed]},
    {ReleaseState, [:repository_changed, :build_changed, :release_changed, :artifact_changed]},
    {AgentInstanceState,
     [
       :agent_instance_changed,
       :repository_changed,
       :repository_ref_changed,
       :run_changed,
       :agent_secret_binding_changed
     ]},
    {RunState, [:run_changed, :review_changed, :artifact_changed]}
  ]

  test "every event-driven page has a deliberate product-event reducer" do
    generated_variants = generated_variants()

    for {module, relevant_variants} <- @coverage,
        variant <- generated_variants do
      effects = module |> ready_state() |> reduce_event(module, variant)

      if variant in relevant_variants do
        assert effects == [:snapshot], "#{inspect(module)} must refresh for #{variant}"
      else
        assert effects == [], "#{inspect(module)} must explicitly ignore #{variant}"
      end
    end
  end

  defp generated_variants do
    ProductEvent.__message_props__().field_props
    |> Map.values()
    |> Enum.filter(&(&1.oneof == 0))
    |> MapSet.new(& &1.name_atom)
  end

  defp ready_state(module) do
    state = new_state(module)

    barrier = %{
      cursor: "coverage-barrier",
      versions: %{},
      schema_version: 1
    }

    {loading, [:snapshot]} =
      module.reduce(
        state,
        {:watch, %{cursor: "coverage-barrier", item: {:snapshot_barrier, barrier}}}
      )

    {ready, []} = ProductEventReducer.snapshot_complete(loading)
    ready
  end

  defp reduce_event(state, module, variant) do
    response = %{
      cursor: "coverage-#{variant}",
      item:
        {:event,
         %{
           id: "event-#{variant}",
           cursor: "coverage-#{variant}",
           aggregate_type: "coverage",
           aggregate_id: Atom.to_string(variant),
           aggregate_version: 1,
           payload: {variant, %{}}
         }}
    }

    {_state, effects} = module.reduce(state, {:watch, response})
    effects
  end

  defp new_state(OrganizationState), do: OrganizationState.new(%{})

  defp new_state(OrganizationWorkspaceState),
    do: OrganizationWorkspaceState.new(%{organization_id: "organization-1"})

  defp new_state(ProjectState), do: ProjectState.new(%{project_id: "project-1"})
  defp new_state(ProjectRunsState), do: ProjectRunsState.new(%{project_id: "project-1"})

  defp new_state(ReleaseState), do: ReleaseState.new("release-1")
  defp new_state(AgentInstanceState), do: AgentInstanceState.new("instance-1")
  defp new_state(RunState), do: RunState.new("run-1")
end
