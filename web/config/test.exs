import Config

# We don't run a server during test. If one is required,
# you can enable the server option below.
config :hephaestus_web, HephaestusWebWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: 4002],
  secret_key_base: "+R53alPytvCbqOc9cgurX1M6Y3dTRs38yZvwHrtuH4/KmgUritHRC4b6IYV0zXPY",
  server: false

# Print only warnings and errors during test
config :logger, level: :warning

# Initialize plugs at runtime for faster test compilation
config :phoenix, :plug_init_mode, :runtime

# Enable helpful, but potentially expensive runtime checks
config :phoenix_live_view,
  enable_expensive_runtime_checks: true

# Sort query params output of verified routes for robust url comparisons
config :phoenix,
  sort_verified_routes_query_params: true

config :hephaestus_web,
  rpc: [
    endpoint: "rpc.test:443",
    mediator_secret: "test-rpc-mediator-secret-that-is-never-transmitted"
  ]
