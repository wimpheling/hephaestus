defmodule HephaestusWeb.MixProject do
  use Mix.Project

  def project do
    [
      app: :hephaestus_web,
      version: "0.1.0",
      elixir: "~> 1.17",
      elixirc_paths: elixirc_paths(Mix.env()),
      start_permanent: Mix.env() == :prod,
      aliases: aliases(),
      hephaestus_architecture: [
        enabled_rules: [
          "WEB-NO-INFRASTRUCTURE-DEPENDENCIES",
          "WEB-RPC-CLIENTS-ONLY-IN-STATE",
          "WEB-NO-HANDWRITTEN-BACKEND-CLIENT",
          "WEB-NO-RAW-BACKEND-ERROR",
          "WEB-NO-FILESYSTEM-OR-PROCESS",
          "UI-RAW-HTML-ONLY-IN-COMPONENTS",
          "UI-TIER-DIRECTION",
          "UI-DECLARED-INTERACTIONS-ONLY",
          "UI-NO-CLASS-ESCAPE-HATCH",
          "UI-DESIGN-TOKENS-ONLY",
          "UI-NO-EXTERNAL-UI-IMPORTS",
          "UI-NO-DOM-INJECTION",
          "UI-PUBLIC-FACADE-COMPLETE",
          "UI-SHOWCASE-AND-TEST-PARITY",
          "UI-PAGE-STATE-COVERAGE",
          "UI-PAGE-COMPANIONS",
          "UI-LIVE-RENDERS-ONE-PAGE",
          "UI-STATE-HAS-NO-HEEX",
          "UI-PAGE-IS-PURE"
        ]
      ],
      deps: deps(),
      compilers: [:phoenix_live_view] ++ Mix.compilers(),
      listeners: [Phoenix.CodeReloader]
    ]
  end

  # Configuration for the OTP application.
  #
  # Type `mix help compile.app` for more information.
  def application do
    [
      mod: {HephaestusWeb.Application, []},
      extra_applications: [:logger, :runtime_tools]
    ]
  end

  def cli do
    [
      preferred_envs: [precommit: :test]
    ]
  end

  # Specifies which paths to compile per environment.
  defp elixirc_paths(:test), do: ["lib", "test/support"]
  defp elixirc_paths(_), do: ["lib"]

  # Specifies your project dependencies.
  #
  # Type `mix help deps` for examples and options.
  defp deps do
    [
      {:phoenix, "~> 1.8.9"},
      {:phoenix_html, "~> 4.1"},
      {:phoenix_live_reload, "~> 1.2", only: :dev},
      {:phoenix_live_view, "~> 1.2.0"},
      {:assent, "~> 0.3.1"},
      {:req, "~> 0.5"},
      {:jose, "~> 1.11"},
      {:protobuf, "== 0.17.0"},
      {:grpc, "== 1.0.2"},
      {:lazy_html, ">= 0.1.0", only: :test},
      {:phoenix_live_dashboard, "~> 0.8.3"},
      {:esbuild, "~> 0.10", runtime: Mix.env() == :dev},
      {:tailwind, "~> 0.5", runtime: Mix.env() == :dev},
      {:telemetry_metrics, "~> 1.0"},
      {:telemetry_poller, "~> 1.0"},
      {:gettext, "~> 1.0"},
      {:jason, "~> 1.2"},
      {:dns_cluster, "~> 0.2.0"},
      {:bandit, "~> 1.5"}
    ]
  end

  # Aliases are shortcuts or tasks specific to the current project.
  # For example, to install project dependencies and perform other setup tasks, run:
  #
  #     $ mix setup
  #
  # See the documentation for `Mix` for more info on aliases.
  defp aliases do
    [
      setup: ["deps.get", "assets.setup", "assets.build"],
      test: ["test"],
      "assets.setup": ["tailwind.install --if-missing", "esbuild.install --if-missing"],
      "assets.build": ["compile", "tailwind hephaestus_web", "esbuild hephaestus_web"],
      "assets.deploy": [
        "tailwind hephaestus_web --minify",
        "esbuild hephaestus_web --minify",
        "phx.digest"
      ],
      precommit: ["compile --warnings-as-errors", "deps.unlock --unused", "format", "test"]
    ]
  end
end
