defmodule HephaestusWeb.UIBrowserTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.UIBrowser

  @generation "10000000-0000-4000-8000-000000000001"
  @projection %{"generation_id" => @generation, "route_base" => "docs"}

  test "builds the deterministic HTTPS bootstrap URL with a 43-character fragment" do
    secret = :crypto.strong_rand_bytes(32)

    assert {:ok, url} =
             UIBrowser.bootstrap_url(@projection, secret, "dark",
               namespace: "ui.example.com",
               platform_host: "example.com"
             )

    uri = URI.parse(url)
    assert uri.scheme == "https"
    assert uri.host == "g-10000000000040008000000000000001.ui.example.com"
    assert uri.port in [nil, 443]
    refute url =~ ":443/_heph/bootstrap"
    assert uri.path == "/_heph/bootstrap"
    assert uri.query == "heph_theme=dark"
    assert byte_size(uri.fragment) == 43
    assert {:ok, ^secret} = Base.url_decode64(uri.fragment, padding: false)
  end

  test "retains an explicit non-default UI port" do
    assert {:ok, url} =
             UIBrowser.bootstrap_url(@projection, :crypto.strong_rand_bytes(32), "light",
               namespace: "ui.example.com",
               platform_host: "example.com",
               port: 8443
             )

    assert URI.parse(url).port == 8443
    assert url =~ ":8443/_heph/bootstrap"
  end

  test "rejects malformed projection, bearer, theme, namespace, and missing config" do
    secret = :crypto.strong_rand_bytes(32)
    options = [namespace: "ui.example.com", platform_host: "example.com"]

    assert {:error, :invalid_projection} =
             UIBrowser.bootstrap_url(
               %{@projection | "generation_id" => "not-a-uuid"},
               secret,
               "light",
               options
             )

    assert {:error, :invalid_projection} =
             UIBrowser.bootstrap_url(
               %{@projection | "route_base" => "../docs"},
               secret,
               "light",
               options
             )

    assert {:error, :invalid_secret} =
             UIBrowser.bootstrap_url(@projection, <<0::248>>, "light", options)

    assert {:error, :invalid_theme} =
             UIBrowser.bootstrap_url(@projection, secret, "system", options)

    assert {:error, :unavailable} =
             UIBrowser.bootstrap_url(@projection, secret, "light",
               namespace: "example.com",
               platform_host: "example.com"
             )

    assert {:error, :unavailable} =
             UIBrowser.bootstrap_url(@projection, secret, "light",
               namespace: "ui.example.com.evil",
               platform_host: "example.com"
             )

    assert {:error, :unavailable} =
             UIBrowser.bootstrap_url(@projection, secret, "light",
               namespace: "ui.example.com",
               platform_host: "example.com",
               port: "0443"
             )

    assert {:error, :unavailable} = UIBrowser.bootstrap_url(@projection, secret, "light")
  end

  test "reserves room for the generated label and accepts atom-keyed projections" do
    namespace =
      Enum.join(
        [
          String.duplicate("a", 63),
          String.duplicate("b", 63),
          String.duplicate("c", 63),
          String.duplicate("d", 14)
        ],
        "."
      ) <> ".example.com"

    assert {:ok, url} =
             UIBrowser.bootstrap_url(
               %{generation_id: @generation, route_base: "docs"},
               :crypto.strong_rand_bytes(32),
               "light",
               namespace: namespace,
               platform_host: "example.com"
             )

    assert URI.parse(url).host =~ "." <> namespace

    too_long_namespace =
      Enum.join(
        [
          String.duplicate("a", 63),
          String.duplicate("b", 63),
          String.duplicate("c", 63),
          String.duplicate("d", 15)
        ],
        "."
      ) <> ".example.com"

    assert {:error, :unavailable} =
             UIBrowser.bootstrap_url(@projection, :crypto.strong_rand_bytes(32), "light",
               namespace: too_long_namespace,
               platform_host: "example.com"
             )
  end
end
