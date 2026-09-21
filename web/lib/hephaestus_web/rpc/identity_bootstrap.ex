defmodule HephaestusWeb.RPC.IdentityBootstrap do
  @moduledoc """
  Browser-OIDC bridge into the generated identity bootstrap RPC.

  Controllers own the browser transaction and session. This transport module
  owns the identity-bootstrap application calls so generated clients remain behind the
  supervised, authenticated RPC boundary.
  """

  alias HephaestusWeb.RPC.Client

  @spec resolve(String.t(), map()) ::
          {:ok, HephaestusWeb.Identity.t()} | {:error, HephaestusWeb.RPC.Error.t()}
  def resolve(issuer, claims), do: Client.resolve_identity(issuer, claims)

  @spec create_session(String.t(), map(), String.t()) ::
          {:ok, map()} | {:error, HephaestusWeb.RPC.Error.t()}
  def create_session(issuer, claims, sid),
    do: Client.create_browser_session(issuer, claims, sid)

  @spec revoke_session(HephaestusWeb.Identity.t()) ::
          {:ok, map()} | {:error, HephaestusWeb.RPC.Error.t()}
  def revoke_session(identity), do: Client.revoke_browser_session(identity)
end
