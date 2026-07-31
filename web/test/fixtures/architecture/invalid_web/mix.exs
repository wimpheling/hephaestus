defmodule InvalidWeb.MixProject do
  use Mix.Project

  def project do
    [
      app: :invalid_web,
      deps: [
        {:ecto_sql, "~> 3.13"},
        {:postgrex, ">= 0.0.0", only: :test},
        {:icons, github: "example/icons", app: false}
      ]
    ]
  end
end
