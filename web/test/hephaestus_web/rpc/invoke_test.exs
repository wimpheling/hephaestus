defmodule HephaestusWeb.RPC.InvokeTest do
  use ExUnit.Case, async: false

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.{Error, Invoke}

  @audience "/hephaestus.projects.v1.ProjectService/GetProject"
  @secret "a-high-entropy-test-secret-that-is-not-transmitted"

  test "passes the deadline and mediator metadata to a native stub" do
    test_process = self()

    stub = fn channel, request, options ->
      send(test_process, {:call, channel, request, options})
      {:ok, %Google.Protobuf.StringValue{value: "response"}}
    end

    request = %Google.Protobuf.StringValue{value: "request"}

    assert {:ok, %Google.Protobuf.StringValue{value: "response"}} =
             Invoke.unary(identity(), @audience, request, stub,
               channel_provider: channel_provider(),
               timeout: 321,
               request_id: "bba97bd2-a9ab-45e3-95fb-df31bef99f08"
             )

    assert_receive {:call, %GRPC.Channel{}, ^request, options}
    assert options[:timeout] == 321
    assert options[:metadata]["x-request-id"] == "bba97bd2-a9ab-45e3-95fb-df31bef99f08"
    assert String.starts_with?(options[:metadata]["authorization"], "Bearer ")
  end

  test "maps timeout and cancellation without exposing backend text" do
    for status <- [:deadline_exceeded, :cancelled] do
      stub = fn _channel, _request, _options ->
        {:error, GRPC.RPCError.exception(status: status, message: "private backend detail")}
      end

      assert {:error, %Error{} = error} =
               Invoke.unary(
                 identity(),
                 @audience,
                 %Google.Protobuf.Empty{},
                 stub,
                 channel_provider: channel_provider()
               )

      refute inspect(error) =~ "private backend detail"
    end
  end

  test "rejects oversized requests and responses" do
    never_called = fn _channel, _request, _options -> flunk("stub must not be called") end
    oversized = %Google.Protobuf.StringValue{value: String.duplicate("x", 20)}

    assert {:error, %Error{kind: :size_limit}} =
             Invoke.unary(identity(), @audience, oversized, never_called,
               maximum_request_bytes: 4,
               channel_provider: channel_provider()
             )

    stub = fn _channel, _request, _options -> {:ok, oversized} end

    assert {:error, %Error{kind: :size_limit}} =
             Invoke.unary(identity(), @audience, %Google.Protobuf.Empty{}, stub,
               maximum_response_bytes: 4,
               channel_provider: channel_provider()
             )
  end

  test "retries an unavailable safe query once using the same request" do
    counter = start_supervised!({Agent, fn -> 0 end}, id: make_ref())
    test_process = self()

    stub = fn _channel, request, _options ->
      attempt = Agent.get_and_update(counter, &{&1, &1 + 1})
      send(test_process, {:attempt, attempt, request})

      if attempt == 0,
        do: {:error, GRPC.RPCError.exception(status: :unavailable)},
        else: {:ok, %Google.Protobuf.Empty{}}
    end

    assert {:ok, %Google.Protobuf.Empty{}} =
             Invoke.unary(identity(), @audience, %Google.Protobuf.Empty{}, stub,
               channel_provider: channel_provider(),
               channel_reset: fn -> send(test_process, :channel_reset) end,
               retry: :safe_query
             )

    assert_receive :channel_reset
    assert_receive {:attempt, 0, %Google.Protobuf.Empty{}}
    assert_receive {:attempt, 1, %Google.Protobuf.Empty{}}
  end

  test "invalidates an unavailable mutation channel without retrying it" do
    test_process = self()

    stub = fn _channel, _request, _options ->
      send(test_process, :mutation_attempt)
      {:error, GRPC.RPCError.exception(status: :unavailable)}
    end

    assert {:error, %Error{kind: :unavailable}} =
             Invoke.unary(
               identity(),
               "/hephaestus.projects.v1.ProjectService/UpdateProject",
               %Google.Protobuf.Empty{},
               stub,
               channel_provider: channel_provider(),
               channel_reset: fn -> send(test_process, :channel_reset) end
             )

    assert_receive :mutation_attempt
    assert_receive :channel_reset
    refute_receive :mutation_attempt
  end

  test "maps an unavailable channel to a typed error without calling the stub" do
    never_called = fn _channel, _request, _options -> flunk("stub must not be called") end

    assert {:error, %Error{kind: :unavailable, retryable: true}} =
             Invoke.unary(identity(), @audience, %Google.Protobuf.Empty{}, never_called,
               channel_provider: fn -> {:error, :connection_refused} end
             )
  end

  test "does not retry a mutation after an unavailable response" do
    counter = start_supervised!({Agent, fn -> 0 end}, id: make_ref())

    stub = fn _channel, _request, _options ->
      Agent.update(counter, &(&1 + 1))
      {:error, GRPC.RPCError.exception(status: :unavailable)}
    end

    assert {:error, %Error{kind: :unavailable}} =
             Invoke.unary(
               identity(),
               "/hephaestus.projects.v1.ProjectService/UpdateProject",
               %Google.Protobuf.Empty{},
               stub,
               channel_provider: channel_provider()
             )

    assert Agent.get(counter, & &1) == 1
  end

  test "bootstrap invocation uses only the verified OIDC bootstrap assertion" do
    test_process = self()
    audience = "/hephaestus.identity.v1.IdentityService/ResolveIdentity"

    attributes = %{
      subject: "external-subject",
      display_name: "Reviewer",
      email: "reviewer@example.test",
      email_verified: true
    }

    stub = fn _channel, request, options ->
      send(test_process, {:bootstrap_call, request, options})
      {:ok, %Google.Protobuf.Empty{}}
    end

    assert {:ok, %Google.Protobuf.Empty{}} =
             Invoke.bootstrap_unary(
               "https://issuer.example",
               attributes,
               audience,
               %Google.Protobuf.Empty{},
               stub,
               channel_provider: channel_provider(),
               request_id: "8da1a7a2-f59f-4f9e-bf7a-44b614dff98a"
             )

    assert_receive {:bootstrap_call, %Google.Protobuf.Empty{}, options}
    assert options[:metadata]["x-request-id"] == "8da1a7a2-f59f-4f9e-bf7a-44b614dff98a"
    assert String.starts_with?(options[:metadata]["authorization"], "Bearer ")
    refute inspect(options) =~ @secret
  end

  defp identity do
    %Identity{
      user_id: "38fa596b-d96f-43c7-a4bc-6ad9f2ce07ad",
      issuer: "https://issuer.example",
      subject: "external-subject",
      display_name: "Reviewer"
    }
  end

  defp channel_provider do
    fn -> {:ok, %GRPC.Channel{host: "rpc.test", port: 443}} end
  end

  setup do
    previous = Application.get_env(:hephaestus_web, :rpc)
    Application.put_env(:hephaestus_web, :rpc, mediator_secret: @secret)

    on_exit(fn ->
      if previous do
        Application.put_env(:hephaestus_web, :rpc, previous)
      else
        Application.delete_env(:hephaestus_web, :rpc)
      end
    end)
  end
end
