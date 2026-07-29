import Config

# config/runtime.exs is executed for all environments, including
# during releases. It is executed after compilation and before the
# system starts, so it is typically used to load production configuration
# and secrets from environment variables or elsewhere. Do not define
# any compile-time configuration in here, as it won't be applied.
# The block below contains prod specific runtime configuration.

# ## Using releases
#
# If you use `mix release`, you need to explicitly enable the server
# by passing the PHX_SERVER=true when you start it:
#
#     PHX_SERVER=true bin/hephaestus_web start
#
# Alternatively, you can use `mix phx.gen.release` to generate a `bin/server`
# script that automatically sets the env var above.
if System.get_env("PHX_SERVER") do
  config :hephaestus_web, HephaestusWebWeb.Endpoint, server: true
end

config :hephaestus_web, HephaestusWebWeb.Endpoint,
  http: [port: String.to_integer(System.get_env("PORT", "4000"))]

if database_url = System.get_env("DATABASE_URL") do
  config :hephaestus_web, HephaestusWeb.Repo,
    url: database_url,
    pool_size: String.to_integer(System.get_env("POOL_SIZE", "10"))

  config :hephaestus_web,
    database_url: database_url
end

config :hephaestus_web,
  artifact_root: System.get_env("HEPHAESTUS_ARTIFACT_ROOT", "/var/lib/hephaestus/artifacts"),
  repository_root:
    System.get_env("HEPHAESTUS_REPOSITORY_ROOT", "/var/lib/hephaestus/repositories"),
  internal_commands: [
    url:
      System.get_env(
        "HEPHAESTUS_INTERNAL_COMMAND_URL",
        "http://127.0.0.1:8080/internal/v1/commands"
      ),
    token:
      System.get_env(
        "HEPHAESTUS_INTERNAL_COMMAND_TOKEN",
        "development-internal-command-token"
      )
  ],
  oidc: [
    issuer: System.get_env("HEPHAESTUS_BROWSER_OIDC_ISSUER", "http://localhost:5556"),
    client_id: System.get_env("HEPHAESTUS_BROWSER_OIDC_CLIENT_ID", "hephaestus-web"),
    client_secret: System.get_env("HEPHAESTUS_BROWSER_OIDC_CLIENT_SECRET", "development-secret"),
    redirect_uri:
      System.get_env(
        "HEPHAESTUS_BROWSER_OIDC_REDIRECT_URI",
        "http://localhost:4000/auth/oidc/callback"
      )
  ]

if config_env() == :dev do
  # Reload browser tabs when matching files change.
  config :hephaestus_web, HephaestusWebWeb.Endpoint,
    live_reload: [
      web_console_logger: true,
      patterns: [
        # Static assets, except user uploads
        ~r"priv/static/(?!uploads/).*\.(js|css|png|jpeg|jpg|gif|svg)$",
        # Gettext translations
        ~r"priv/gettext/.*\.po$",
        # Router, Controllers, LiveViews and LiveComponents
        ~r"lib/hephaestus_web_web/router\.ex$",
        ~r"lib/hephaestus_web_web/(controllers|live|components)/.*\.(ex|heex)$"
      ]
    ]
end

if config_env() == :prod do
  database_url =
    System.get_env("DATABASE_URL") ||
      raise """
      environment variable DATABASE_URL is missing.
      For example: ecto://USER:PASS@HOST/DATABASE
      """

  maybe_ipv6 = if System.get_env("ECTO_IPV6") in ~w(true 1), do: [:inet6], else: []

  config :hephaestus_web, HephaestusWeb.Repo,
    # ssl: true,
    url: database_url,
    pool_size: String.to_integer(System.get_env("POOL_SIZE") || "10"),
    # For machines with several cores, consider starting multiple pools of `pool_size`
    # pool_count: 4,
    socket_options: maybe_ipv6

  # The secret key base is used to sign/encrypt cookies and other secrets.
  # A default value is used in config/dev.exs and config/test.exs but you
  # want to use a different value for prod and you most likely don't want
  # to check this value into version control, so we use an environment
  # variable instead.
  secret_key_base =
    System.get_env("SECRET_KEY_BASE") ||
      raise """
      environment variable SECRET_KEY_BASE is missing.
      You can generate one by calling: mix phx.gen.secret
      """

  host = System.get_env("PHX_HOST") || "example.com"

  config :hephaestus_web, :dns_cluster_query, System.get_env("DNS_CLUSTER_QUERY")

  config :hephaestus_web, HephaestusWebWeb.Endpoint,
    url: [host: host, port: 443, scheme: "https"],
    http: [
      # Enable IPv6 and bind on all interfaces.
      # Set it to  {0, 0, 0, 0, 0, 0, 0, 1} for local network only access.
      # See the documentation on https://bandit.hexdocs.pm/Bandit.html#t:options/0
      # for details about using IPv6 vs IPv4 and loopback vs public addresses.
      ip: {0, 0, 0, 0, 0, 0, 0, 0}
    ],
    secret_key_base: secret_key_base

  # ## SSL Support
  #
  # To get SSL working, you will need to add the `https` key
  # to your endpoint configuration:
  #
  #     config :hephaestus_web, HephaestusWebWeb.Endpoint,
  #       https: [
  #         ...,
  #         port: 443,
  #         cipher_suite: :strong,
  #         keyfile: System.get_env("SOME_APP_SSL_KEY_PATH"),
  #         certfile: System.get_env("SOME_APP_SSL_CERT_PATH")
  #       ]
  #
  # The `cipher_suite` is set to `:strong` to support only the
  # latest and more secure SSL ciphers. This means old browsers
  # and clients may not be supported. You can set it to
  # `:compatible` for wider support.
  #
  # `:keyfile` and `:certfile` expect an absolute path to the key
  # and cert in disk or a relative path inside priv, for example
  # "priv/ssl/server.key". For all supported SSL configuration
  # options, see https://plug.hexdocs.pm/Plug.SSL.html#configure/1
  #
  # We also recommend setting `force_ssl` in your config/prod.exs,
  # ensuring no data is ever sent via http, always redirecting to https:
  #
  #     config :hephaestus_web, HephaestusWebWeb.Endpoint,
  #       force_ssl: [hsts: true]
  #
  # Check `Plug.SSL` for all available options in `force_ssl`.
end
