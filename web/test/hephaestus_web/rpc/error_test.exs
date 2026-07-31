defmodule HephaestusWeb.RPC.ErrorTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.RPC.Error
  alias Hephaestus.Common.V1.{Diagnostic, ErrorDetail, OpaqueId}

  test "maps authorization, timeout, cancellation, and size failures" do
    assert %Error{kind: :permission_denied, retryable: false} =
             Error.from_rpc(GRPC.RPCError.exception(status: :permission_denied))

    assert %Error{kind: :timeout, retryable: true} =
             Error.from_rpc(GRPC.RPCError.exception(status: :deadline_exceeded))

    assert %Error{kind: :cancelled, retryable: false} =
             Error.from_rpc(GRPC.RPCError.exception(status: :cancelled))

    assert %Error{kind: :size_limit, retryable: false} =
             Error.from_rpc(GRPC.RPCError.exception(status: :resource_exhausted))
  end

  test "never presents unrestricted backend error messages" do
    sentinel = "SENSITIVE_BACKEND_ERROR_SENTINEL"

    error =
      GRPC.RPCError.exception(
        status: :invalid_argument,
        message: sentinel
      )
      |> Error.from_rpc()

    refute inspect(error) =~ sentinel
    refute Error.present(error) =~ sentinel
    assert Error.present(error) == "The submitted values were not accepted."
  end

  test "decodes only the generated error detail and strips free-form text" do
    sentinel = "SENSITIVE_BACKEND_ERROR_SENTINEL"

    detail = %ErrorDetail{
      code: :ERROR_CODE_INVALID_ARGUMENT,
      reason: sentinel,
      request_id: %OpaqueId{value: "16752a97-60f3-4122-bd67-c30ae4dd3641"},
      retryable: false,
      diagnostics: [%Diagnostic{field: "name", message: sentinel}]
    }

    any = %Google.Protobuf.Any{
      type_url: "type.googleapis.com/hephaestus.common.v1.ErrorDetail",
      value: ErrorDetail.encode(detail)
    }

    error =
      GRPC.RPCError.exception(status: :invalid_argument, message: sentinel, details: [any])
      |> Error.from_rpc()

    assert %ErrorDetail{reason: "", diagnostics: [%Diagnostic{message: ""}]} = error.detail
    assert error.detail.request_id.value == "16752a97-60f3-4122-bd67-c30ae4dd3641"
    refute inspect(error) =~ sentinel
  end

  test "discards unknown typed detail envelopes" do
    unknown = %Google.Protobuf.Any{
      type_url: "type.googleapis.com/private.Detail",
      value: "secret"
    }

    assert %Error{detail: nil} =
             Error.from_rpc(
               GRPC.RPCError.exception(status: :invalid_argument, details: [unknown])
             )
  end
end
