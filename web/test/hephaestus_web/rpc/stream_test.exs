defmodule HephaestusWeb.RPC.StreamTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.{Error, Stream}

  @audience "/hephaestus.events.v1.ProductEventService/WatchProject"

  test "consumes generated server-stream messages with mediator metadata and closes its channel" do
    test_process = self()
    request = %Google.Protobuf.StringValue{value: "project"}

    stub = fn channel, ^request, options ->
      send(test_process, {:stub, channel, options})

      {:ok,
       [
         {:headers, %{}},
         {:ok, %Google.Protobuf.StringValue{value: "first"}},
         {:ok, %Google.Protobuf.StringValue{value: "second"}},
         {:trailers, %{}}
       ]}
    end

    consumer = fn message ->
      send(test_process, {:message, message})
      :cont
    end

    assert :ok =
             Stream.consume(identity(), @audience, request, stub, consumer,
               channel_provider: channel_provider(),
               channel_close: channel_close(test_process)
             )

    assert_receive {:stub, %GRPC.Channel{}, options}
    assert options[:timeout] == :infinity
    assert String.starts_with?(options[:metadata]["authorization"], "Bearer ")
    assert_receive {:message, %Google.Protobuf.StringValue{value: "first"}}
    assert_receive {:message, %Google.Protobuf.StringValue{value: "second"}}
    assert_receive {:closed, %GRPC.Channel{}}
  end

  test "halts without consuming later messages and still closes the channel" do
    test_process = self()

    responses =
      Elixir.Stream.map(["first", "second"], fn value ->
        send(test_process, {:enumerated, value})
        {:ok, %Google.Protobuf.StringValue{value: value}}
      end)

    stub = fn _channel, _request, _options -> {:ok, responses} end

    assert :ok =
             Stream.consume(
               identity(),
               @audience,
               %Google.Protobuf.Empty{},
               stub,
               fn _message -> :halt end,
               channel_provider: channel_provider(),
               channel_close: channel_close(test_process)
             )

    assert_receive {:enumerated, "first"}
    refute_receive {:enumerated, "second"}
    assert_receive {:closed, %GRPC.Channel{}}
  end

  test "rejects oversized requests and individual stream messages" do
    never_called = fn _channel, _request, _options -> flunk("stub must not be called") end
    oversized = %Google.Protobuf.StringValue{value: String.duplicate("x", 32)}

    assert {:error, %Error{kind: :size_limit}} =
             Stream.consume(identity(), @audience, oversized, never_called, fn _ -> :cont end,
               maximum_request_bytes: 4
             )

    stub = fn _channel, _request, _options -> {:ok, [{:ok, oversized}]} end

    assert {:error, %Error{kind: :size_limit}} =
             Stream.consume(
               identity(),
               @audience,
               %Google.Protobuf.Empty{},
               stub,
               fn _ -> flunk("oversized message must not reach consumer") end,
               maximum_message_bytes: 4,
               channel_provider: channel_provider(),
               channel_close: fn _ -> :ok end
             )
  end

  test "maps stream trailer failures without retaining backend text" do
    error =
      GRPC.RPCError.exception(status: :permission_denied, message: "private authorization text")

    stub = fn _channel, _request, _options -> {:ok, [{:error, error}]} end

    assert {:error, %Error{kind: :permission_denied} = result} =
             Stream.consume(
               identity(),
               @audience,
               %Google.Protobuf.Empty{},
               stub,
               fn _ -> :cont end,
               channel_provider: channel_provider(),
               channel_close: fn _ -> :ok end
             )

    refute inspect(result) =~ "private authorization text"
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

  defp channel_close(owner) do
    fn channel ->
      send(owner, {:closed, channel})
      :ok
    end
  end

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
