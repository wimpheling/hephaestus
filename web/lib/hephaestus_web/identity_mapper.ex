defmodule HephaestusWeb.IdentityMapper do
  @moduledoc """
  Maps verified OIDC `(issuer, subject)` pairs to immutable internal users.
  """

  alias HephaestusWeb.{Identity, Repo}

  def map_verified(issuer, %{"sub" => subject} = claims)
      when is_binary(issuer) and is_binary(subject) do
    Repo.transaction(fn ->
      result =
        Ecto.Adapters.SQL.query!(
          Repo,
          """
          SELECT users.id, users.display_name
          FROM external_identities identity
          JOIN users ON users.id = identity.user_id
          WHERE identity.issuer = $1 AND identity.subject = $2
            AND users.status = 'active'
          """,
          [issuer, subject]
        )

      case result.rows do
        [[user_id, display_name]] ->
          Ecto.Adapters.SQL.query!(
            Repo,
            """
            INSERT INTO user_profiles (user_id, validated_claims)
            VALUES ($1, $2)
            ON CONFLICT (user_id) DO UPDATE
            SET validated_claims = EXCLUDED.validated_claims,
                updated_at = now()
            """,
            [user_id, claims]
          )

          %Identity{
            user_id: Ecto.UUID.load!(user_id),
            issuer: issuer,
            subject: subject,
            display_name: display_name
          }

        [] ->
          Repo.rollback(:identity_not_registered)
      end
    end)
  end

  def map_verified(_issuer, _claims), do: {:error, :invalid_subject}
end
