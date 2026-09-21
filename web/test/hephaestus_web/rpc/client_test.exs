defmodule HephaestusWeb.RPC.ClientTest do
  use ExUnit.Case, async: true

  alias Hephaestus.Common.V1.{
    Cursor,
    MutationReceipt,
    NetworkPolicy,
    OpaqueId,
    ParameterValue,
    RuntimePolicy
  }

  alias Hephaestus.Identity.V1.CreateBrowserSessionResponse
  alias Hephaestus.Instance.V1.ImportAgentResponse
  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.Client

  @user_id "10000000-0000-4000-8000-000000000001"
  @session_id "20000000-0000-4000-8000-000000000002"
  @event_id "30000000-0000-4000-8000-000000000003"

  test "serializes session import ids, parameter, and broker-only policy" do
    caller = self()
    model_rule_id = "40000000-0000-4000-8000-000000000004"

    stub = fn _channel, request, _options ->
      send(caller, {:import_call, request})

      {:ok,
       %ImportAgentResponse{
         instance_id: %OpaqueId{value: "50000000-0000-4000-8000-000000000005"},
         revision_id: %OpaqueId{value: "60000000-0000-4000-8000-000000000006"}
       }}
    end

    assert {:ok,
            %{
              "instance_id" => "50000000-0000-4000-8000-000000000005",
              "revision_id" => "60000000-0000-4000-8000-000000000006"
            }} =
             Client.import_agent(
               identity(),
               "70000000-0000-4000-8000-000000000007",
               "80000000-0000-4000-8000-000000000008",
               "session-chat",
               %{"model_rule_id" => model_rule_id},
               %{"vcpus" => 1, "memory_mib" => 256, "network" => "broker_only"},
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert_receive {:import_call, request}
    assert request.project_id.value == "70000000-0000-4000-8000-000000000007"
    assert request.release_agent_id.value == "80000000-0000-4000-8000-000000000008"
    assert request.name == "session-chat"

    assert [%ParameterValue{name: "model_rule_id", value: {:string_value, ^model_rule_id}}] =
             request.parameters

    assert %RuntimePolicy{
             vcpus: 1,
             memory_mib: 256,
             network: network
           } = request.selected_policy

    assert network == NetworkPolicy.value(:NETWORK_POLICY_BROKER_ONLY)
  end

  test "projects a typed create response into safe session metadata" do
    response = response()

    assert {:ok,
            %{
              user_id: @user_id,
              session_id: @session_id,
              expires_at: %DateTime{},
              receipt: %{
                "committed_cursor" => "cursor-1",
                "aggregate_version" => 1,
                "event_id" => @event_id
              }
            }} = Client.project_created_session_response(response)
  end

  test "rejects malformed generated fields without raising" do
    response = response()

    assert {:error, %HephaestusWeb.RPC.Error{kind: :invalid}} =
             Client.project_created_session_response(%{user_id: nil})

    assert {:error, %HephaestusWeb.RPC.Error{kind: :invalid}} =
             Client.project_created_session_response(%{
               response
               | expires_at: %Google.Protobuf.Timestamp{},
                 receipt: %MutationReceipt{committed_cursor: nil}
             })
  end

  defp response do
    %CreateBrowserSessionResponse{
      user_id: %OpaqueId{value: @user_id},
      session_id: %OpaqueId{value: @session_id},
      expires_at: %Google.Protobuf.Timestamp{
        seconds: System.system_time(:second) + 3600,
        nanos: 0
      },
      receipt: %MutationReceipt{
        committed_cursor: %Cursor{value: "cursor-1"},
        aggregate_version: 1,
        event_id: %OpaqueId{value: @event_id}
      }
    }
  end

  defp identity do
    %Identity{
      user_id: "90000000-0000-4000-8000-000000000009",
      issuer: "https://issuer.example",
      subject: "subject",
      display_name: "Test User",
      sid: "a0000000-0000-4000-8000-00000000000a"
    }
  end

  defp channel_provider, do: fn -> {:ok, %GRPC.Channel{}} end
end
