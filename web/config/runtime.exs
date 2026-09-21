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

platform_origin_env = System.get_env("HEPHAESTUS_PLATFORM_HTTPS_ORIGIN")

platform_fallback_host = System.get_env("HEPHAESTUS_PLATFORM_HOST") || System.get_env("PHX_HOST")

platform_origin_pattern =
  ~r/\Ahttps:\/\/(?<host>[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*)(?::(?<port>[1-9][0-9]{0,4}))?\z/

canonical_origin = fn %URI{host: host, port: port} ->
  host = String.downcase(host)
  port = if port in [nil, 443], do: "", else: ":#{port}"
  "https://#{host}#{port}"
end

{platform_host, platform_origin} =
  case platform_origin_env do
    nil ->
      {platform_fallback_host,
       if(is_binary(platform_fallback_host),
         do: "https://#{String.downcase(platform_fallback_host)}",
         else: nil
       )}

    explicit_origin ->
      parsed_origin =
        try do
          URI.parse(explicit_origin)
        rescue
          URI.ParseError -> nil
        end

      valid_origin? =
        case {Regex.named_captures(platform_origin_pattern, explicit_origin), parsed_origin} do
          {%{"host" => raw_host, "port" => raw_port},
           %URI{
             scheme: "https",
             userinfo: nil,
             host: parsed_host,
             port: parsed_port,
             path: path,
             query: nil,
             fragment: nil
           }} ->
            expected_port =
              case raw_port do
                nil -> 443
                "" -> 443
                value -> String.to_integer(value)
              end

            is_binary(parsed_host) and byte_size(raw_host) <= 253 and
              String.downcase(parsed_host) == String.downcase(raw_host) and
              parsed_port == expected_port and path in [nil, ""] and
              expected_port in 1..65_535

          _invalid ->
            false
        end

      if valid_origin? do
        %URI{host: host} = parsed_origin
        {String.downcase(host), canonical_origin.(parsed_origin)}
      else
        raise "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN must be an exact HTTPS origin"
      end
  end

config :hephaestus_web,
  rpc: [
    endpoint: System.get_env("HEPHAESTUS_RPC_ENDPOINT", "127.0.0.1:8080"),
    mediator_secret:
      System.get_env(
        "HEPHAESTUS_RPC_MEDIATOR_SECRET",
        "development-rpc-mediator-secret-change-before-deployment"
      )
  ],
  ui_browser: [
    namespace: System.get_env("HEPHAESTUS_UI_NAMESPACE"),
    platform_origin: platform_origin,
    platform_host: platform_host,
    port: System.get_env("HEPHAESTUS_UI_PORT")
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
  ],
  git_http: [
    origin: System.get_env("HEPHAESTUS_GIT_HTTP_ORIGIN", "http://127.0.0.1:8080")
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

  endpoint_url =
    case platform_origin_env do
      nil ->
        [host: System.get_env("PHX_HOST") || "example.com", port: 443, scheme: "https"]

      _explicit_origin ->
        %URI{scheme: scheme, host: host, port: port} = URI.parse(platform_origin)
        [host: host, port: port || 443, scheme: scheme]
    end

  config :hephaestus_web, :dns_cluster_query, System.get_env("DNS_CLUSTER_QUERY")

  config :hephaestus_web, HephaestusWebWeb.Endpoint,
    url: endpoint_url,
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
