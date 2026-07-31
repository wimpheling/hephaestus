defmodule HephaestusWeb.RPC.IdentityBootstrap do
  @moduledoc """
  Browser-OIDC bridge into the generated identity bootstrap RPC.

  Controllers own the browser transaction and session. This transport module
  owns the sole application call so generated clients remain behind the
  supervised, authenticated RPC boundary.
  """

  alias HephaestusWeb.RPC.Client

  @spec resolve(String.t(), map()) ::
          {:ok, HephaestusWeb.Identity.t()} | {:error, HephaestusWeb.RPC.Error.t()}
  def resolve(issuer, claims), do: Client.resolve_identity(issuer, claims)
end
