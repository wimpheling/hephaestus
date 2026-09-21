defmodule HephaestusWeb.RPC.MintLifecycleTest do
  use ExUnit.Case, async: false

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.{Channel, Error, Invoke}

  @deadline_ms 50
  @rpc_timeout_ms 1_000
  @audience "/hephaestus.projects.v1.ProjectService/GetProject"
  @mutation_audience "/hephaestus.projects.v1.ProjectService/UpdateProject"
  @secret "a-high-entropy-test-secret-that-is-not-transmitted"

  defmodule Service do
    use GRPC.Service, name: "hephaestus.test.MintLifecycle"

    rpc(:Ping, Google.Protobuf.Empty, Google.Protobuf.Empty)
    rpc(:Fail, Google.Protobuf.Empty, Google.Protobuf.Empty)
  end

  defmodule Stub do
    use GRPC.Stub, service: Service
  end

  defmodule DelayedServer do
    @response_delay_ms 250
    use Plug.Router

    plug(:match)
    plug(:dispatch)

    match _ do
      {:ok, _body, conn} = Plug.Conn.read_body(conn)

      send(
        :persistent_term.get({__MODULE__, :test_process}),
        {:server_request, conn.request_path}
      )

      Process.sleep(@response_delay_ms)

      if String.ends_with?(conn.request_path, "/Fail") do
        conn
        |> Plug.Conn.put_resp_header("content-type", "application/grpc+proto")
        |> Plug.Conn.put_resp_header("grpc-status", "14")
        |> Plug.Conn.send_resp(200, <<0, 0, 0, 0, 0>>)
      else
        conn
        |> Plug.Conn.put_resp_header("content-type", "application/grpc+proto")
        |> Plug.Conn.put_resp_header("grpc-status", "0")
        |> Plug.Conn.send_resp(200, <<0, 0, 0, 0, 0>>)
      end
    end
  end

  setup do
    {:ok, _apps} = Application.ensure_all_started(:grpc)
    previous_rpc = Application.get_env(:hephaestus_web, :rpc)
    Application.put_env(:hephaestus_web, :rpc, mediator_secret: @secret)
    :persistent_term.put({DelayedServer, :test_process}, self())

    {:ok, server} = Bandit.start_link(plug: DelayedServer, port: 0, startup_log: false)
    Process.unlink(server)
    {:ok, {_address, port}} = ThousandIsland.listener_info(server)

    {:ok, request_supervisor} = Task.Supervisor.start_link()
    Process.unlink(request_supervisor)

    {:ok, channel_server} =
      Channel.start_link(
        name: nil,
        configuration: [endpoint: "127.0.0.1:#{port}", reconnect_attempts: 0],
        connector: GRPC.Stub
      )

    Process.unlink(channel_server)

    on_exit(fn ->
      if previous_rpc do
        Application.put_env(:hephaestus_web, :rpc, previous_rpc)
      else
        Application.delete_env(:hephaestus_web, :rpc)
      end

      :persistent_term.erase({DelayedServer, :test_process})
      _ = GenServer.stop(channel_server)
      _ = Supervisor.stop(request_supervisor)
      _ = GenServer.stop(server)
    end)

    %{channel_server: channel_server, request_supervisor: request_supervisor}
  end

  test "a canceled Invoke caller cannot poison concurrent Mint requests",
       %{channel_server: channel_server, request_supervisor: request_supervisor} do
    assert {:ok, %GRPC.Channel{adapter_payload: %{conn_pid: connection_pid}}} =
             Channel.get(channel_server)

    test_process = self()

    caller =
      spawn(fn ->
        send(test_process, :cancellable_started)

        Invoke.unary(
          identity(),
          @audience,
          %Google.Protobuf.Empty{},
          &Stub.ping/3,
          channel_server: channel_server,
          request_supervisor: request_supervisor,
          timeout: @rpc_timeout_ms
        )
      end)

    caller_ref = Process.monitor(caller)
    assert_receive :cancellable_started
    assert_receive {:server_request, "/hephaestus.test.MintLifecycle/Ping"}, 1_000
    Process.exit(caller, :application_deadline)
    assert_receive {:DOWN, ^caller_ref, :process, ^caller, :application_deadline}, 1_000

    concurrent =
      Task.async(fn ->
        Invoke.unary(
          identity(),
          @audience,
          %Google.Protobuf.Empty{},
          &Stub.ping/3,
          channel_server: channel_server,
          request_supervisor: request_supervisor,
          timeout: @rpc_timeout_ms
        )
      end)

    assert {:ok, %Google.Protobuf.Empty{}} = Task.await(concurrent, 2_000)
    assert Process.alive?(connection_pid)
    assert eventually(fn -> Task.Supervisor.children(request_supervisor) == [] end)
  end

  test "a bounded deadline leaves the worker to drain and preserves the shared connection",
       %{channel_server: channel_server, request_supervisor: request_supervisor} do
    assert {:ok, %GRPC.Channel{adapter_payload: %{conn_pid: connection_pid}}} =
             Channel.get(channel_server)

    assert {:error, %Error{kind: :timeout}} =
             Invoke.unary(
               identity(),
               @audience,
               %Google.Protobuf.Empty{},
               &Stub.ping/3,
               channel_server: channel_server,
               request_supervisor: request_supervisor,
               timeout: @deadline_ms
             )

    assert eventually(fn -> Task.Supervisor.children(request_supervisor) == [] end)
    assert Process.alive?(connection_pid)

    assert {:ok, %Google.Protobuf.Empty{}} =
             Invoke.unary(
               identity(),
               @audience,
               %Google.Protobuf.Empty{},
               &Stub.ping/3,
               channel_server: channel_server,
               request_supervisor: request_supervisor,
               timeout: @rpc_timeout_ms
             )
  end

  test "a genuine unavailable response resets the channel and the next call recovers",
       %{channel_server: channel_server, request_supervisor: request_supervisor} do
    channel_reset = fn -> Channel.reset(channel_server) end

    assert {:ok, %GRPC.Channel{adapter_payload: %{conn_pid: old_connection_pid}}} =
             Channel.get(channel_server)

    assert {:error, %Error{kind: :unavailable}} =
             Invoke.unary(
               identity(),
               @mutation_audience,
               %Google.Protobuf.Empty{},
               &Stub.fail/3,
               channel_server: channel_server,
               request_supervisor: request_supervisor,
               channel_reset: channel_reset,
               timeout: @rpc_timeout_ms
             )

    assert_receive {:server_request, "/hephaestus.test.MintLifecycle/Fail"}, 1_000
    refute_receive {:server_request, "/hephaestus.test.MintLifecycle/Fail"}, 100
    assert eventually(fn -> not Process.alive?(old_connection_pid) end)

    assert {:ok, %Google.Protobuf.Empty{}} =
             Invoke.unary(
               identity(),
               @audience,
               %Google.Protobuf.Empty{},
               &Stub.ping/3,
               channel_server: channel_server,
               request_supervisor: request_supervisor,
               timeout: @rpc_timeout_ms
             )
  end

  defp eventually(predicate, attempts \\ 20)

  defp eventually(_predicate, 0), do: false

  defp eventually(predicate, attempts) do
    if predicate.() do
      true
    else
      Process.sleep(25)
      eventually(predicate, attempts - 1)
    end
  end

  defp identity do
    %Identity{
      user_id: "38fa596b-d96f-43c7-a4bc-6adf2fce07ad",
      issuer: "https://issuer.example",
      subject: "external-subject",
      display_name: "Reviewer",
      sid: "20000000-0000-4000-8000-000000000002",
      session_expires_at: 4_000_000_000
    }
  end
end
