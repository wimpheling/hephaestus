defmodule HephaestusWeb.RPC.Mediator do
  @moduledoc """
  Creates short-lived, audience-bound assertions for the trusted RPC boundary.

  The configured shared secret is only key material. It is domain-separated
  before signing and is never transmitted as a bearer credential.
  """

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.UUID

  @domain_separator "hephaestus-rpc-mediator-v1\0"
  @issuer "hephaestus-web-mediator"
  @maximum_lifetime_seconds 30

  @type metadata :: [{String.t(), String.t()}]

  @doc "Returns authorization and request-correlation metadata for one RPC."
  @spec metadata(Identity.t(), String.t(), keyword()) :: metadata()
  def metadata(%Identity{} = identity, audience, options \\ []) do
    request_id = Keyword.get_lazy(options, :request_id, &UUID.generate/0)
    token = assertion(identity, audience, options)

    [
      {"authorization", "Bearer " <> token},
      {"x-request-id", request_id}
    ]
  end

  @doc "Returns metadata for the one verified-OIDC identity bootstrap RPC."
  @spec bootstrap_metadata(String.t(), map(), String.t(), keyword()) :: metadata()
  def bootstrap_metadata(issuer, attributes, audience, options \\ [])
      when is_binary(issuer) and is_map(attributes) do
    request_id = Keyword.get_lazy(options, :request_id, &UUID.generate/0)
    token = bootstrap_assertion(issuer, attributes, audience, options)

    [
      {"authorization", "Bearer " <> token},
      {"x-request-id", request_id}
    ]
  end

  @doc "Signs an assertion scoped to one exact generated RPC method."
  @spec assertion(Identity.t(), String.t(), keyword()) :: String.t()
  def assertion(%Identity{user_id: user_id}, audience, options \\ []) do
    validate_audience!(audience)

    %{
      "iss" => @issuer,
      "aud" => audience,
      "sub" => user_id,
      "jti" => Keyword.get_lazy(options, :jti, &UUID.generate/0)
    }
    |> sign(options)
  end

  @doc false
  @spec bootstrap_assertion(String.t(), map(), String.t(), keyword()) :: String.t()
  def bootstrap_assertion(issuer, attributes, audience, options \\ [])
      when is_binary(issuer) and is_map(attributes) do
    validate_audience!(audience)

    claims = %{
      "iss" => @issuer,
      "aud" => audience,
      "sub" => "hephaestus-web-mediator",
      "jti" => Keyword.get_lazy(options, :jti, &UUID.generate/0),
      "actor_kind" => "verified_oidc_bootstrap",
      "oidc_iss" => issuer,
      "oidc_sub" => Map.fetch!(attributes, :subject),
      "name" => Map.fetch!(attributes, :display_name),
      "email" => Map.fetch!(attributes, :email),
      "email_verified" => Map.fetch!(attributes, :email_verified)
    }

    sign(claims, options)
  end

  defp sign(claims, options) do
    now = Keyword.get_lazy(options, :now, fn -> System.system_time(:second) end)
    lifetime = Keyword.get(options, :lifetime_seconds, @maximum_lifetime_seconds)
    validate_lifetime!(lifetime)

    claims =
      Map.merge(claims, %{
        "iat" => now,
        "nbf" => now,
        "exp" => now + lifetime
      })

    secret = Keyword.get_lazy(options, :secret, &configured_secret!/0)
    validate_secret!(secret)
    signing_key = :crypto.hash(:sha256, @domain_separator <> secret)
    jwk = JOSE.JWK.from_oct(signing_key)

    {_headers, compact} =
      jwk
      |> JOSE.JWT.sign(%{"alg" => "HS256", "typ" => "JWT"}, claims)
      |> JOSE.JWS.compact()

    compact
  end

  defp configured_secret! do
    :hephaestus_web
    |> Application.fetch_env!(:rpc)
    |> Keyword.fetch!(:mediator_secret)
  end

  defp validate_audience!(audience) when is_binary(audience) do
    if Regex.match?(
         ~r{\A/hephaestus\.[a-z][a-z0-9_]*\.v1\.[A-Z][A-Za-z0-9]*/[A-Z][A-Za-z0-9]*\z},
         audience
       ) do
      :ok
    else
      raise ArgumentError, "RPC audience must be one exact versioned method path"
    end
  end

  defp validate_audience!(_audience),
    do: raise(ArgumentError, "RPC audience must be one exact versioned method path")

  defp validate_lifetime!(lifetime)
       when is_integer(lifetime) and lifetime > 0 and lifetime <= @maximum_lifetime_seconds,
       do: :ok

  defp validate_lifetime!(_lifetime),
    do: raise(ArgumentError, "mediator assertion lifetime must be between 1 and 30 seconds")

  defp validate_secret!(secret) when is_binary(secret) and byte_size(secret) >= 32, do: :ok

  defp validate_secret!(_secret),
    do: raise(ArgumentError, "RPC mediator secret must contain at least 32 bytes")
end
