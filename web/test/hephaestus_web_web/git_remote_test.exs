defmodule HephaestusWebWeb.GitRemoteTest do
  use ExUnit.Case, async: false

  alias HephaestusWebWeb.GitRemote

  setup do
    previous_configuration = Application.fetch_env!(:hephaestus_web, :git_http)
    Application.put_env(:hephaestus_web, :git_http, origin: "https://git.test:9443")

    on_exit(fn -> Application.put_env(:hephaestus_web, :git_http, previous_configuration) end)
  end

  test "uses the configured Git HTTP origin instead of the browser endpoint" do
    assert GitRemote.url("repository-1") == "https://git.test:9443/repository-1"
  end

  test "preserves a configured Git HTTP path prefix" do
    assert GitRemote.url("https://git.example/forge", "repository-1") ==
             "https://git.example/forge/repository-1"
  end

  test "rejects an origin that could embed credentials or alter the repository route" do
    for origin <- [
          "git.example",
          "ftp://git.example",
          "https://token@git.example",
          "https://git.example?repository=other",
          "https://git.example#other"
        ] do
      assert_raise ArgumentError, ~r/HEPHAESTUS_GIT_HTTP_ORIGIN/, fn ->
        GitRemote.url(origin, "repository-1")
      end
    end
  end

  test "builds a copyable clone command with a shell-safe destination" do
    assert GitRemote.clone_command(
             %{"name" => "Cooking Agent; rm -rf /"},
             "https://git.example/repository-1"
           ) == "git clone https://heph-pat@git.example/repository-1 cooking-agent-rm-rf"
  end
end
