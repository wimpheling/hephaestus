defmodule HephaestusWeb.Identity do
  @moduledoc """
  Minimal browser principal reconstructed from the encrypted Phoenix session.

  Tokens never enter the LiveView process or database query layer.
  """

  @enforce_keys [:user_id, :issuer, :subject, :display_name]
  defstruct [:user_id, :issuer, :subject, :display_name]

  @uuid_pattern ~r/\A[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/i

  @type t :: %__MODULE__{
          user_id: String.t(),
          issuer: String.t(),
          subject: String.t(),
          display_name: String.t()
        }

  def from_session(%{
        "user_id" => user_id,
        "issuer" => issuer,
        "subject" => subject,
        "display_name" => display_name
      }) do
    if is_binary(user_id) and Regex.match?(@uuid_pattern, user_id) do
      {:ok,
       %__MODULE__{
         user_id: String.downcase(user_id),
         issuer: issuer,
         subject: subject,
         display_name: display_name
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
      "display_name" => identity.display_name
    }
  end
end
