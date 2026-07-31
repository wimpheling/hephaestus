defmodule HephaestusWeb.RPC.MediatorTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.Mediator

  @secret "a-high-entropy-test-secret-that-is-not-transmitted"
  @audience "/hephaestus.projects.v1.ProjectService/GetProject"
  @user_id "38fa596b-d96f-43c7-a4bc-6ad9f2ce07ad"

  test "assertion is audience-bound, short-lived, and contains no external identity" do
    token =
      Mediator.assertion(identity(), @audience,
        secret: @secret,
        now: 1_700_000_000,
        jti: "bf1e332e-d37d-4c11-8f4f-da2f68143de7"
      )

    signing_key =
      :crypto.hash(:sha256, "hephaestus-rpc-mediator-v1\0" <> @secret)
      |> JOSE.JWK.from_oct()

    assert {true, jwt, _jws} = JOSE.JWT.verify_strict(signing_key, ["HS256"], token)

    assert jwt.fields == %{
             "iss" => "hephaestus-web-mediator",
             "aud" => @audience,
             "sub" => @user_id,
             "jti" => "bf1e332e-d37d-4c11-8f4f-da2f68143de7",
             "iat" => 1_700_000_000,
             "nbf" => 1_700_000_000,
             "exp" => 1_700_000_030
           }

    refute token =~ identity().issuer
    refute token =~ identity().subject
  end

  test "metadata never transmits the configured secret" do
    metadata =
      Mediator.metadata(identity(), @audience,
        secret: @secret,
        now: 1_700_000_000,
        request_id: "9b88cf56-83a1-43a4-9f03-3ae0bf8919e6"
      )

    assert [{"authorization", "Bearer " <> token}, {"x-request-id", request_id}] = metadata
    assert request_id == "9b88cf56-83a1-43a4-9f03-3ae0bf8919e6"
    refute token == @secret
    refute inspect(metadata) =~ @secret
  end

  test "bootstrap assertion binds every typed verified OIDC field" do
    audience = "/hephaestus.identity.v1.IdentityService/ResolveIdentity"

    attributes = %{
      subject: "external-subject",
      display_name: "Reviewer",
      email: "reviewer@example.test",
      email_verified: true
    }

    token =
      Mediator.bootstrap_assertion("https://issuer.example", attributes, audience,
        secret: @secret,
        now: 1_700_000_000,
        jti: "a1f089fe-8b84-43f0-8633-ad77f89aeb38"
      )

    signing_key =
      :crypto.hash(:sha256, "hephaestus-rpc-mediator-v1\0" <> @secret)
      |> JOSE.JWK.from_oct()

    assert {true, jwt, _jws} = JOSE.JWT.verify_strict(signing_key, ["HS256"], token)

    assert jwt.fields["sub"] == "hephaestus-web-mediator"
    assert jwt.fields["actor_kind"] == "verified_oidc_bootstrap"
    assert jwt.fields["oidc_iss"] == "https://issuer.example"
    assert jwt.fields["oidc_sub"] == attributes.subject
    assert jwt.fields["name"] == attributes.display_name
    assert jwt.fields["email"] == attributes.email
    assert jwt.fields["email_verified"] == attributes.email_verified
  end

  test "assertion rejects broad audiences and excessive lifetimes" do
    assert_raise ArgumentError, fn ->
      Mediator.assertion(identity(), "hephaestus.projects.v1.ProjectService", secret: @secret)
    end

    assert_raise ArgumentError, fn ->
      Mediator.assertion(identity(), @audience, secret: @secret, lifetime_seconds: 31)
    end

    assert_raise ArgumentError, fn ->
      Mediator.assertion(identity(), @audience, secret: "short")
    end
  end

  defp identity do
    %Identity{
      user_id: @user_id,
      issuer: "https://issuer.example",
      subject: "external-subject",
      display_name: "Reviewer"
    }
  end
end
