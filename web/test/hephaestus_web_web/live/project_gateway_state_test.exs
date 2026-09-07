defmodule HephaestusWebWeb.ProjectGatewayStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{ProductEventReducer, ProjectGatewayState, ProjectGatewaysState}

  @covered_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  test "covers the gateway detail lifecycle contract" do
    assert @covered_statuses == ProjectGatewayState.statuses()
    assert ProjectGatewayState.stream_mode() == :page_scoped
  end

  test "gateway list is loaded through one finite authorized snapshot" do
    state = ProjectGatewaysState.new(%{project_id: "project-1"})

    {loading, [:load]} = ProjectGatewaysState.reduce(state, {:load, 1})

    {ready, []} =
      ProjectGatewaysState.reduce(loading, {
        :loaded,
        1,
        [%{"id" => "gateway-1", "lifecycle" => "enabled"}]
      })

    assert %{status: :ready, gateways: [%{"id" => "gateway-1"}]} =
             ProjectGatewaysState.present(ready)
  end

  test "gateway detail waits for its lifecycle receipt before refreshing" do
    state =
      barrier_ready(ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"}))

    {waiting, []} =
      ProjectGatewayState.reduce(state, {
        :transitioned,
        %{
          "gateway" => %{"id" => "gateway-1", "lifecycle" => "paused"},
          "receipt" => %{
            "committed_cursor" => "cursor-2",
            "event_id" => "gateway-event",
            "aggregate_version" => 2
          }
        }
      })

    assert waiting.status == :stale

    {_changed, [:snapshot]} =
      ProjectGatewayState.reduce(
        waiting,
        {:watch, %{event(:gateway_changed, "gateway") | cursor: "cursor-2"}}
      )
  end

  test "gateway lifecycle rejection reaches an error presentation state" do
    state = ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"})

    {submitting, [{:lifecycle, "paused"}]} =
      ProjectGatewayState.reduce(state, {:lifecycle, "paused"})

    {failed, []} = ProjectGatewayState.reduce(submitting, :lifecycle_failed)

    assert failed.status == :error
    assert ProjectGatewayState.present(failed).status == :error
  end

  test "configuration keeps the active revision CAS target and waits for its receipt" do
    state = ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"})

    gateway = %{
      "id" => "gateway-1",
      "active_revision_id" => "revision-1",
      "revisions" => [
        %{
          "id" => "revision-1",
          "release_id" => "release-1",
          "release_agent_id" => "agent-1",
          "secret_slots" => []
        }
      ],
      "lifecycle" => "enabled"
    }

    {ready, []} =
      ProjectGatewayState.reduce(
        state,
        {:loaded, 0, gateway, [], [%{"id" => "agent-1", "parameter_schema" => []}],
         %{"imports" => []}, []}
      )

    {submitting, [{:configure, attributes}]} =
      ProjectGatewayState.reduce(
        ready,
        {:configure, %{"parameters" => %{}, "secret_selections" => %{}}}
      )

    assert submitting.status == :submitting
    assert attributes["parameters"] == %{}

    {waiting, []} =
      ProjectGatewayState.reduce(submitting, {
        :configured,
        0,
        %{
          "revision_id" => "revision-2",
          "receipt" => %{
            "committed_cursor" => "cursor-2",
            "event_id" => "gateway-event",
            "aggregate_version" => 2
          }
        }
      })

    assert waiting.status == :stale

    assert {ready, []} =
             ProjectGatewayState.reduce(
               ready,
               {:configured, 1, %{"receipt" => %{"committed_cursor" => "late"}}}
             )

    assert ready.status == :ready
  end

  test "binder-only state retains authorized organization imports without secret listing" do
    state = ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"})

    gateway = %{
      "id" => "gateway-1",
      "active_revision_id" => "revision-1",
      "revisions" => [%{"id" => "revision-1", "release_agent_id" => "agent-1"}],
      "lifecycle" => "enabled"
    }

    {ready, []} =
      ProjectGatewayState.reduce(
        state,
        {:loaded, 0, gateway, [], [%{"id" => "agent-1", "parameter_schema" => []}],
         %{
           "imports" => [
             %{
               "id" => "org-import-1",
               "secret_id" => "org-secret-1",
               "alias" => "Organization inbound",
               "state" => "active",
               "active_version_id" => "org-version-7"
             }
           ]
         }, []}
      )

    presentation = ProjectGatewayState.present(ready)
    assert presentation.secret_authority["imports"]
    assert presentation.secrets == []
  end

  test "binding uses only the active revision slots and waits for its receipt" do
    state = ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"})

    gateway = %{
      "id" => "gateway-1",
      "active_revision_id" => "revision-2",
      "revisions" => [
        %{"id" => "revision-2", "mailbox_slots" => ["deliver"]},
        %{"id" => "revision-1", "mailbox_slots" => ["old"]}
      ],
      "lifecycle" => "enabled"
    }

    {ready, []} =
      ProjectGatewayState.reduce(
        state,
        {:loaded, 0, gateway, [], [], %{"imports" => []}, [],
         [
           %{
             "id" => "instance-1",
             "name" => "Worker",
             "state" => "ready",
             "mailbox_id" => "mailbox-1"
           }
         ], []}
      )

    assert ProjectGatewayState.present(ready).binding_slots == ["deliver"]

    {submitting, [{:bind, attributes}]} =
      ProjectGatewayState.reduce(
        ready,
        {:bind,
         %{
           "slot_key" => "deliver",
           "mailbox_id" => "mailbox-1",
           "producer_id" => "project-gateway"
         }}
      )

    assert submitting.status == :submitting
    assert attributes["mailbox_id"] == "mailbox-1"
    assert ProjectGatewayState.present(submitting).configure_form == %{}
    assert ProjectGatewayState.present(submitting).binding_form == attributes

    {waiting, []} =
      ProjectGatewayState.reduce(submitting, {
        :bound,
        0,
        %{
          "binding_id" => "binding-1",
          "receipt" => %{
            "committed_cursor" => "cursor-2",
            "event_id" => "gateway-event",
            "aggregate_version" => 2
          }
        }
      })

    assert waiting.status == :stale
  end

  defp barrier_ready(state) do
    barrier = %{cursor: "cursor-1", versions: %{}, schema_version: 1}

    {loading, [:snapshot]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: "cursor-1", item: {:snapshot_barrier, barrier}},
        [:gateway_changed]
      )

    {ready, []} = ProductEventReducer.snapshot_complete(loading)
    ready
  end

  defp event(kind, aggregate_type) do
    %{
      cursor: "cursor-event",
      item:
        {:event,
         %{
           id: "gateway-event",
           cursor: "cursor-event",
           aggregate_type: aggregate_type,
           aggregate_id: "gateway-1",
           aggregate_version: 1,
           payload: {kind, %{}}
         }}
    }
  end
end
