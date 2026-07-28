defmodule HephaestusWeb.Repo do
  use Ecto.Repo,
    otp_app: :hephaestus_web,
    adapter: Ecto.Adapters.Postgres
end
