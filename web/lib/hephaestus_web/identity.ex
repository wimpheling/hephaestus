defmodule HephaestusWeb.Identity do
  @moduledoc """
  Minimal browser principal reconstructed from the signed Phoenix session.

  OIDC tokens never enter the LiveView process or database query layer. The
  durable session ID is a credential and is redacted from inspection output.
  """

  @enforce_keys [:user_id, :issuer, :subject, :display_name]
  @derive {Inspect, except: [:sid]}
  defstruct [:user_id, :issuer, :subject, :display_name, :sid, :session_expires_at]

  @uuid_pattern ~r/\A[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/i

  @type t :: %__MODULE__{
          user_id: String.t(),
          issuer: String.t(),
          subject: String.t(),
          display_name: String.t(),
          sid: String.t() | nil,
          session_expires_at: integer() | nil
        }

  def from_session(%{
        "user_id" => user_id,
        "issuer" => issuer,
        "subject" => subject,
        "display_name" => display_name,
        "sid" => sid,
        "session_expires_at" => session_expires_at
      }) do
    if valid_session_id?(user_id) and valid_session_id?(sid) and
         valid_session_expiry?(session_expires_at) do
      {:ok,
       %__MODULE__{
         user_id: String.downcase(user_id),
         issuer: issuer,
         subject: subject,
         display_name: display_name,
         sid: String.downcase(sid),
         session_expires_at: session_expires_at
       }}
    else
      :error
    end
  end

  def from_session(_session), do: :error

  def to_session(%__MODULE__{} = identity) do
    %{
      "user_id" => identity.user_id,
      "issuer" => identity.issuer,
      "subject" => identity.subject,
      "display_name" => identity.display_name,
      "sid" => identity.sid,
      "session_expires_at" => identity.session_expires_at
    }
  end

  @doc "Attaches a newly created durable session after validating the RPC response."
  @spec attach_session(t(), String.t(), map()) :: {:ok, t()} | {:error, :invalid_response}
  def attach_session(
        %__MODULE__{} = identity,
        sid,
        %{
          user_id: user_id,
          session_id: session_id,
          expires_at: %DateTime{} = expires_at,
          receipt: receipt
        }
      ) do
    if valid_session_id?(sid) and
         valid_session_id?(user_id) and
         String.downcase(user_id) == identity.user_id and
         valid_session_id?(session_id) and
         valid_receipt?(receipt) and
         DateTime.compare(expires_at, DateTime.utc_now()) == :gt do
      {:ok,
       %{
         identity
         | sid: String.downcase(sid),
           session_expires_at: DateTime.to_unix(expires_at)
       }}
    else
      {:error, :invalid_response}
    end
  end

  def attach_session(_identity, _sid, _response), do: {:error, :invalid_response}

  @doc "Checks the RFC 4122 UUID representation used for browser SIDs."
  @spec valid_session_id?(term()) :: boolean()
  def valid_session_id?(value), do: is_binary(value) and Regex.match?(@uuid_pattern, value)

  defp valid_receipt?(%{
         "committed_cursor" => cursor,
         "aggregate_version" => version,
         "event_id" => event_id
       }) do
    is_binary(cursor) and cursor != "" and is_integer(version) and version > 0 and
      valid_session_id?(event_id)
  end

  defp valid_receipt?(_receipt), do: false

  defp valid_session_expiry?(value) when is_integer(value),
    do: value > System.system_time(:second)

  defp valid_session_expiry?(_value), do: false
end
