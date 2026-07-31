defmodule HephaestusWeb.RPC.Error do
  @moduledoc """
  Stable Phoenix-side error categories for backend RPC failures.

  Backend messages are deliberately discarded. Presentation code receives only
  these reviewed categories and optional generated typed details.
  """

  @enforce_keys [:kind, :retryable]
  defstruct [:kind, :retryable, :detail]

  @type kind ::
          :cancelled
          | :conflict
          | :invalid
          | :not_found
          | :permission_denied
          | :precondition
          | :size_limit
          | :timeout
          | :unauthenticated
          | :unavailable

  @type t :: %__MODULE__{kind: kind(), retryable: boolean(), detail: struct() | nil}

  @cancelled GRPC.Status.cancelled()
  @invalid_argument GRPC.Status.invalid_argument()
  @deadline_exceeded GRPC.Status.deadline_exceeded()
  @not_found GRPC.Status.not_found()
  @already_exists GRPC.Status.already_exists()
  @permission_denied GRPC.Status.permission_denied()
  @resource_exhausted GRPC.Status.resource_exhausted()
  @failed_precondition GRPC.Status.failed_precondition()
  @aborted GRPC.Status.aborted()
  @unauthenticated GRPC.Status.unauthenticated()
  @unavailable GRPC.Status.unavailable()
  @error_detail_type "type.googleapis.com/hephaestus.common.v1.ErrorDetail"

  @doc "Maps a transport failure without retaining its unrestricted message."
  @spec from_rpc(GRPC.RPCError.t()) :: t()
  def from_rpc(%GRPC.RPCError{status: @cancelled, details: details}),
    do: error(:cancelled, false, details)

  def from_rpc(%GRPC.RPCError{status: @invalid_argument, details: details}),
    do: error(:invalid, false, details)

  def from_rpc(%GRPC.RPCError{status: @deadline_exceeded, details: details}),
    do: error(:timeout, true, details)

  def from_rpc(%GRPC.RPCError{status: @not_found, details: details}),
    do: error(:not_found, false, details)

  def from_rpc(%GRPC.RPCError{status: @already_exists, details: details}),
    do: error(:conflict, false, details)

  def from_rpc(%GRPC.RPCError{status: @permission_denied, details: details}),
    do: error(:permission_denied, false, details)

  def from_rpc(%GRPC.RPCError{status: @resource_exhausted, details: details}),
    do: error(:size_limit, false, details)

  def from_rpc(%GRPC.RPCError{status: @failed_precondition, details: details}),
    do: error(:precondition, false, details)

  def from_rpc(%GRPC.RPCError{status: @aborted, details: details}),
    do: error(:conflict, true, details)

  def from_rpc(%GRPC.RPCError{status: @unauthenticated, details: details}),
    do: error(:unauthenticated, false, details)

  def from_rpc(%GRPC.RPCError{status: @unavailable, details: details}),
    do: error(:unavailable, true, details)

  def from_rpc(%GRPC.RPCError{details: details}), do: error(:unavailable, false, details)

  @doc "Maps channel, process, and adapter failures to one safe category."
  @spec unavailable() :: t()
  def unavailable, do: %__MODULE__{kind: :unavailable, retryable: true}

  @doc false
  @spec local(kind(), boolean()) :: t()
  def local(kind, retryable \\ false), do: %__MODULE__{kind: kind, retryable: retryable}

  @doc "Returns reviewed user-facing copy without backend error text."
  @spec present(t()) :: String.t()
  def present(%__MODULE__{kind: :cancelled}), do: "The operation was cancelled."
  def present(%__MODULE__{kind: :conflict}), do: "The resource changed; refresh and try again."
  def present(%__MODULE__{kind: :invalid}), do: "The submitted values were not accepted."
  def present(%__MODULE__{kind: :not_found}), do: "The resource is no longer available."

  def present(%__MODULE__{kind: :permission_denied}),
    do: "Access to this resource was revoked."

  def present(%__MODULE__{kind: :precondition}),
    do: "The operation is not valid in the resource's current state."

  def present(%__MODULE__{kind: :size_limit}), do: "The requested content is too large."
  def present(%__MODULE__{kind: :timeout}), do: "The service did not respond in time."
  def present(%__MODULE__{kind: :unauthenticated}), do: "Your session is no longer authorized."
  def present(%__MODULE__{kind: :unavailable}), do: "The service is temporarily unavailable."

  defp error(kind, retryable, details) do
    %__MODULE__{kind: kind, retryable: retryable, detail: typed_detail(details)}
  end

  defp typed_detail(details) when is_list(details) do
    Enum.find_value(details, fn
      %Google.Protobuf.Any{type_url: @error_detail_type, value: value} ->
        value
        |> Hephaestus.Common.V1.ErrorDetail.decode()
        |> sanitize_detail()

      _unknown ->
        nil
    end)
  rescue
    Protobuf.DecodeError -> nil
  end

  defp typed_detail(_details), do: nil

  defp sanitize_detail(detail) do
    diagnostics = Enum.map(detail.diagnostics, &%{&1 | message: ""})
    %{detail | reason: "", diagnostics: diagnostics}
  end
end
