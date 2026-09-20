defmodule HephaestusWeb.RPC.ClientTest do
  use ExUnit.Case, async: true

  alias Hephaestus.Common.V1.{Cursor, MutationReceipt, OpaqueId}
  alias Hephaestus.Identity.V1.CreateBrowserSessionResponse
  alias HephaestusWeb.RPC.Client

  @user_id "10000000-0000-4000-8000-000000000001"
  @session_id "20000000-0000-4000-8000-000000000002"
  @event_id "30000000-0000-4000-8000-000000000003"

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
end
