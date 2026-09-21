defmodule HephaestusWeb.RuntimeConfigTest do
  use ExUnit.Case, async: false

  @runtime_path Path.expand("../config/runtime.exs", __DIR__)
  @environment_keys [
    "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN",
    "HEPHAESTUS_PLATFORM_HOST",
    "PHX_HOST",
    "SECRET_KEY_BASE",
    "PHX_SERVER"
  ]

  test "explicit mixed-case custom origin drives the production endpoint URL" do
    config =
      read_runtime(%{
        "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN" => "https://Platform.Example.Test:8443",
        "PHX_HOST" => "legacy.example.test",
        "SECRET_KEY_BASE" => String.duplicate("a", 64)
      })

    assert endpoint_url(config) == [host: "platform.example.test", port: 8443, scheme: "https"]

    assert ui_browser(config)[:platform_origin] == "https://platform.example.test:8443"
    assert ui_browser(config)[:platform_host] == "platform.example.test"
  end

  test "explicit default HTTPS port is canonicalized consistently" do
    config =
      read_runtime(%{
        "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN" => "https://Platform.Example.Test:443",
        "SECRET_KEY_BASE" => String.duplicate("b", 64)
      })

    assert endpoint_url(config) == [host: "platform.example.test", port: 443, scheme: "https"]
    assert ui_browser(config)[:platform_origin] == "https://platform.example.test"
  end

  test "malformed explicit origin fails before production configuration is returned" do
    assert_raise RuntimeError,
                 "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN must be an exact HTTPS origin",
                 fn ->
                   read_runtime(%{
                     "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN" => "https://platform.example.test/path",
                     "SECRET_KEY_BASE" => String.duplicate("c", 64)
                   })
                 end
  end

  test "production endpoint keeps the legacy PHX_HOST fallback without explicit origin" do
    config =
      read_runtime(%{
        "PHX_HOST" => "legacy.example.test",
        "SECRET_KEY_BASE" => String.duplicate("d", 64)
      })

    assert endpoint_url(config) == [host: "legacy.example.test", port: 443, scheme: "https"]
  end

  defp read_runtime(values) do
    previous = Map.new(@environment_keys, &{&1, System.get_env(&1)})

    try do
      Enum.each(@environment_keys, &System.delete_env/1)
      Enum.each(values, fn {key, value} -> System.put_env(key, value) end)
      Config.Reader.read!(@runtime_path, env: :prod)
    after
      Enum.each(previous, fn
        {key, nil} -> System.delete_env(key)
        {key, value} -> System.put_env(key, value)
      end)
    end
  end

  defp endpoint_url(config) do
    config
    |> Keyword.fetch!(:hephaestus_web)
    |> Keyword.fetch!(HephaestusWebWeb.Endpoint)
    |> Keyword.fetch!(:url)
  end

  defp ui_browser(config) do
    config
    |> Keyword.fetch!(:hephaestus_web)
    |> Keyword.fetch!(:ui_browser)
  end
end
