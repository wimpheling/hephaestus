defmodule HephaestusWeb.RPC.ProductEvents do
  @moduledoc """
  Native generated-client adapter for authorized scoped product-event watches.

  Generated messages are normalized here into a small typed page-event shape;
  protobuf values do not cross into LiveView callbacks or presentation code.
  """

  alias Hephaestus.Common.V1.{Cursor, OpaqueId}

  alias Hephaestus.Event.V1.{
    ProductEventService,
    WatchAgentInstanceRequest,
    WatchIdentityRequest,
    WatchOrganizationRequest,
    WatchProjectRequest,
    WatchRepositoryRequest,
    WatchRunRequest
  }

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.{Projection, Stream}

  @type scope ::
          :identity
          | {:organization, String.t()}
          | {:project, String.t()}
          | {:repository, String.t()}
          | {:run, String.t()}
          | {:agent_instance, String.t()}

  @type item ::
          {:snapshot_barrier, map()}
          | {:event, map()}
          | {:retention_gap, map()}
          | {:access_revoked, map()}

  @type response :: %{sequence: non_neg_integer(), cursor: String.t() | nil, item: item()}

  @doc "Consumes one authorized scope stream from an exact committed cursor."
  @spec watch(Identity.t(), scope(), String.t() | nil, (response() -> :cont | :halt), keyword()) ::
          :ok | {:error, HephaestusWeb.RPC.Error.t()}
  def watch(%Identity{} = identity, scope, committed_cursor, consumer, options \\ [])
      when is_function(consumer, 1) do
    {audience, request, stub_call} = invocation(scope, committed_cursor)
    stub_call = Keyword.get(options, :stub_call, stub_call)

    Stream.consume(
      identity,
      audience,
      request,
      stub_call,
      fn response -> consumer.(project_response(response)) end,
      Keyword.merge(
        [maximum_request_bytes: 4_096, maximum_message_bytes: 262_144],
        options
      )
    )
  end

  defp invocation(:identity, committed_cursor) do
    {
      "/hephaestus.event.v1.ProductEventService/WatchIdentity",
      %WatchIdentityRequest{resume_cursor: cursor(committed_cursor)},
      &ProductEventService.Stub.watch_identity/3
    }
  end

  defp invocation({:organization, id}, committed_cursor) do
    {
      "/hephaestus.event.v1.ProductEventService/WatchOrganization",
      %WatchOrganizationRequest{organization_id: id(id), resume_cursor: cursor(committed_cursor)},
      &ProductEventService.Stub.watch_organization/3
    }
  end

  defp invocation({:project, id}, committed_cursor) do
    {
      "/hephaestus.event.v1.ProductEventService/WatchProject",
      %WatchProjectRequest{project_id: id(id), resume_cursor: cursor(committed_cursor)},
      &ProductEventService.Stub.watch_project/3
    }
  end

  defp invocation({:repository, id}, committed_cursor) do
    {
      "/hephaestus.event.v1.ProductEventService/WatchRepository",
      %WatchRepositoryRequest{repository_id: id(id), resume_cursor: cursor(committed_cursor)},
      &ProductEventService.Stub.watch_repository/3
    }
  end

  defp invocation({:run, id}, committed_cursor) do
    {
      "/hephaestus.event.v1.ProductEventService/WatchRun",
      %WatchRunRequest{run_id: id(id), resume_cursor: cursor(committed_cursor)},
      &ProductEventService.Stub.watch_run/3
    }
  end

  defp invocation({:agent_instance, id}, committed_cursor) do
    {
      "/hephaestus.event.v1.ProductEventService/WatchAgentInstance",
      %WatchAgentInstanceRequest{
        agent_instance_id: id(id),
        resume_cursor: cursor(committed_cursor)
      },
      &ProductEventService.Stub.watch_agent_instance/3
    }
  end

  defp project_response(%{sequence: sequence, committed_cursor: cursor, item: item}) do
    %{sequence: sequence, cursor: cursor_value(cursor), item: project_item(item)}
  end

  defp project_item({:snapshot_barrier, barrier}) do
    versions =
      Map.new(barrier.aggregate_versions, fn reference ->
        {{Projection.to_value(reference.aggregate_type), id_value(reference.aggregate_id)},
         reference.aggregate_version}
      end)

    {:snapshot_barrier,
     %{
       cursor: cursor_value(barrier.committed_cursor),
       schema_version: barrier.schema_version,
       versions: versions
     }}
  end

  defp project_item({:event, event}) do
    {payload_kind, payload} = event.payload

    {:event,
     %{
       id: id_value(event.event_id),
       cursor: cursor_value(event.cursor),
       aggregate_type: Projection.to_value(event.aggregate_type),
       aggregate_id: id_value(event.aggregate_id),
       aggregate_version: event.aggregate_version,
       schema_version: event.schema_version,
       payload: {payload_kind, Projection.to_value(payload)}
     }}
  end

  defp project_item({:retention_gap, gap}) do
    {:retention_gap,
     %{
       requested_cursor: cursor_value(gap.requested_cursor),
       earliest_available_cursor: cursor_value(gap.earliest_available_cursor),
       latest_committed_cursor: cursor_value(gap.latest_committed_cursor)
     }}
  end

  defp project_item({:access_revoked, revoked}) do
    {:access_revoked, %{observed_at: Projection.to_value(revoked.observed_at)}}
  end

  defp cursor(nil), do: nil
  defp cursor(""), do: nil
  defp cursor(value), do: %Cursor{value: value}
  defp cursor_value(nil), do: nil
  defp cursor_value(%Cursor{value: ""}), do: nil
  defp cursor_value(%Cursor{value: value}), do: value
  defp id(value), do: %OpaqueId{value: value}
  defp id_value(nil), do: nil
  defp id_value(%OpaqueId{value: ""}), do: nil
  defp id_value(%OpaqueId{value: value}), do: value
end
