defmodule HephaestusWeb.Application do
  # See https://elixir.hexdocs.pm/Application.html
  # for more information on OTP Applications
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    children = [
      HephaestusWebWeb.Telemetry,
      {DNSCluster, query: Application.get_env(:hephaestus_web, :dns_cluster_query) || :ignore},
      {Phoenix.PubSub, name: HephaestusWeb.PubSub},
      HephaestusWeb.RPC.Channel,
      {Task.Supervisor, name: HephaestusWeb.PageTaskSupervisor},
      # Start to serve requests, typically the last entry
      HephaestusWebWeb.Endpoint
    ]

    # See https://elixir.hexdocs.pm/Supervisor.html
    # for other strategies and supported options
    opts = [strategy: :one_for_one, name: HephaestusWeb.Supervisor]
    Supervisor.start_link(children, opts)
  end

  # Tell Phoenix to update the endpoint configuration
  # whenever the application is updated.
  @impl true
  def config_change(changed, _new, removed) do
    HephaestusWebWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
