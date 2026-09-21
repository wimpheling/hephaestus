defmodule HephaestusWebWeb.SessionOptions do
  @moduledoc false

  @signing_salt "ptlYZxM4"

  @doc "Returns the cookie session options for the selected build profile."
  @spec for_profile(:production | :development | :test) :: keyword()
  def for_profile(:production) do
    common_options()
    |> Keyword.merge(
      key: "__Host-hephaestus_web_key",
      path: "/",
      secure: true,
      http_only: true
    )
  end

  def for_profile(profile) when profile in [:development, :test] do
    common_options()
    |> Keyword.merge(
      key: "_hephaestus_web_key",
      path: "/",
      secure: false,
      http_only: true
    )
  end

  defp common_options do
    [
      store: :cookie,
      signing_salt: @signing_salt,
      same_site: "Lax"
    ]
  end
end
