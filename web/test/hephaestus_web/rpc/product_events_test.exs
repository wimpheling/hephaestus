defmodule HephaestusWeb.RPC.ProductEventsTest do
  use ExUnit.Case, async: true

  alias Hephaestus.Common.V1.{Cursor, OpaqueId}

  alias Hephaestus.Event.V1.{
    AggregateVersionReference,
    ProductEvent,
    ProjectChanged,
    ScopeSnapshotBarrier,
    WatchProjectRequest,
    WatchProjectResponse
  }

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.ProductEvents

  test "builds an exact scoped resume request and projects barriers and typed events" do
    test_process = self()

    stub = fn _channel, %WatchProjectRequest{} = request, options ->
      send(test_process, {:request, request, options})

      {:ok,
       [
         {:ok,
          %WatchProjectResponse{
            sequence: 1,
            committed_cursor: %Cursor{value: "cursor-7"},
            item:
              {:snapshot_barrier,
               %ScopeSnapshotBarrier{
                 committed_cursor: %Cursor{value: "cursor-7"},
                 aggregate_versions: [
                   %AggregateVersionReference{
                     aggregate_type: :AGGREGATE_TYPE_PROJECT,
                     aggregate_id: id("project-1"),
                     aggregate_version: 3
                   }
                 ],
                 schema_version: 1
               }}
          }},
         {:ok,
          %WatchProjectResponse{
            sequence: 2,
            committed_cursor: %Cursor{value: "cursor-8"},
            item:
              {:event,
               %ProductEvent{
                 event_id: id("event-1"),
                 cursor: %Cursor{value: "cursor-8"},
                 aggregate_type: :AGGREGATE_TYPE_PROJECT,
                 aggregate_id: id("project-1"),
                 aggregate_version: 4,
                 schema_version: 1,
                 payload:
                   {:project_changed,
                    %ProjectChanged{
                      change: :CHANGE_KIND_UPDATED,
                      state: :LIFECYCLE_STATE_ACTIVE
                    }}
               }}
          }}
       ]}
    end

    consumer = fn response ->
      send(test_process, {:response, response})
      :cont
    end

    assert :ok =
             ProductEvents.watch(
               identity(),
               {:project, "project-1"},
               "cursor-6",
               consumer,
               channel_provider: channel_provider(),
               channel_close: fn _ -> :ok end,
               stub_call: stub
             )

    assert_receive {:request,
                    %WatchProjectRequest{
                      project_id: %OpaqueId{value: "project-1"},
                      resume_cursor: %Cursor{value: "cursor-6"}
                    }, options}

    assert String.starts_with?(options[:metadata]["authorization"], "Bearer ")

    assert_receive {:response,
                    %{
                      sequence: 1,
                      cursor: "cursor-7",
                      item:
                        {:snapshot_barrier,
                         %{
                           cursor: "cursor-7",
                           versions: %{{"project", "project-1"} => 3}
                         }}
                    }}

    assert_receive {:response,
                    %{
                      sequence: 2,
                      cursor: "cursor-8",
                      item:
                        {:event,
                         %{
                           id: "event-1",
                           aggregate_type: "project",
                           aggregate_version: 4,
                           payload:
                             {:project_changed, %{"change" => "updated", "state" => "active"}}
                         }}
                    }}
  end

  defp id(value), do: %OpaqueId{value: value}

  defp identity do
    %Identity{
      user_id: "38fa596b-d96f-43c7-a4bc-6ad9f2ce07ad",
      issuer: "https://issuer.example",
      subject: "external-subject",
      display_name: "Reviewer"
    }
  end

  defp channel_provider,
    do: fn -> {:ok, %GRPC.Channel{host: "rpc.test", port: 443}} end

  setup do
    previous = Application.get_env(:hephaestus_web, :rpc)

    Application.put_env(:hephaestus_web, :rpc,
      mediator_secret: "a-high-entropy-test-secret-that-is-not-transmitted"
    )

    on_exit(fn ->
      if previous,
        do: Application.put_env(:hephaestus_web, :rpc, previous),
        else: Application.delete_env(:hephaestus_web, :rpc)
    end)
  end
end
