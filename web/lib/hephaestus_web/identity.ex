defmodule HephaestusWeb.Identity do
  @moduledoc """
  Minimal browser principal reconstructed from the encrypted Phoenix session.

  Tokens never enter the LiveView process or database query layer.
  """

  @enforce_keys [:user_id, :issuer, :subject, :display_name]
  defstruct [:user_id, :issuer, :subject, :display_name]

  def from_session(%{
        "user_id" => user_id,
        "issuer" => issuer,
        "subject" => subject,
        "display_name" => display_name
      }) do
    case Ecto.UUID.cast(user_id) do
      {:ok, canonical_id} ->
        {:ok,
         %__MODULE__{
           user_id: canonical_id,
           issuer: issuer,
           subject: subject,
           display_name: display_name
         }}

      :error ->
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
