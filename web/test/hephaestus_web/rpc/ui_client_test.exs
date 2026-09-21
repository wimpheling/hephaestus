defmodule HephaestusWeb.RPC.UiClientTest do
  use ExUnit.Case, async: true

  alias Hephaestus.Common.V1.{OpaqueId, PageResponse}

  alias Hephaestus.Release.V1.{
    CreateUiBrowserHandoffResponse,
    GlobalUiInstallationTarget,
    InstallUiResponse,
    ListUiInstallationsResponse,
    ReleaseUiIcon,
    ReleaseUiPresentation,
    UiInstallationContentKind,
    UiInstallationLifecycle,
    UiInstallationNavigation,
    UiInstallationTarget
  }

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.Client
  alias HephaestusWeb.UIBrowser

  @organization "10000000-0000-4000-8000-000000000001"
  @installation "20000000-0000-4000-8000-000000000002"
  @generation "30000000-0000-4000-8000-000000000003"

  test "list sends explicit organization, target, opaque cursor, and safe-query retry" do
    caller = self()

    stub = fn _channel, request, options ->
      send(caller, {:list_call, request, options})

      {:ok,
       %ListUiInstallationsResponse{
         installations: [],
         page: %PageResponse{next_page_token: "opaque|cursor"}
       }}
    end

    assert {:ok, %{"installations" => [], "page" => %{"next_page_token" => "opaque|cursor"}}} =
             Client.list_ui_installations(
               identity(),
               @organization,
               {:project, @installation},
               "opaque|cursor",
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert_receive {:list_call, request, options}
    assert request.organization_id.value == @organization

    assert %UiInstallationTarget{target: {:project_id, %OpaqueId{value: @installation}}} =
             request.target

    assert request.page.page_size == 100
    assert request.page.page_token == "opaque|cursor"
    assert options[:metadata]["x-request-id"] =~ ~r/\A[0-9a-f-]{36}\z/i
  end

  test "list rejects an unsupported target before making an RPC" do
    assert {:error, %HephaestusWeb.RPC.Error{kind: :invalid}} =
             Client.list_ui_installations(
               identity(),
               @organization,
               {:organization, @organization}
             )
  end

  test "list encodes global and repository targets as distinct oneofs" do
    caller = self()

    stub = fn _channel, request, _options ->
      send(caller, {:target, request.target})
      {:ok, %ListUiInstallationsResponse{}}
    end

    assert {:ok, _} =
             Client.list_ui_installations(identity(), @organization, :global, "",
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert {:ok, _} =
             Client.list_ui_installations(
               identity(),
               @organization,
               {:repository, @installation},
               "",
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert_receive {:target, %UiInstallationTarget{target: {:global, _}}}

    assert_receive {:target,
                    %UiInstallationTarget{
                      target: {:repository_id, %OpaqueId{value: @installation}}
                    }}
  end

  test "list projects every target shape and navigation enum into safe values" do
    navigation = fn target ->
      %UiInstallationNavigation{
        installation_id: %OpaqueId{value: @installation},
        generation_id: %OpaqueId{value: @generation},
        organization_id: %OpaqueId{value: @organization},
        target: target,
        lifecycle: UiInstallationLifecycle.value(:UI_INSTALLATION_LIFECYCLE_ENABLED),
        release_id: %OpaqueId{value: "70000000-0000-4000-8000-000000000007"},
        ui_key: "admin",
        label: "Admin",
        icon: ReleaseUiIcon.value(:RELEASE_UI_ICON_APP),
        presentation: ReleaseUiPresentation.value(:RELEASE_UI_PRESENTATION_IFRAME),
        route_base: "docs",
        content_kind: UiInstallationContentKind.value(:UI_INSTALLATION_CONTENT_KIND_STATIC),
        launchable: true
      }
    end

    response = %ListUiInstallationsResponse{
      installations: [
        navigation.(%UiInstallationTarget{target: {:global, %GlobalUiInstallationTarget{}}}),
        navigation.(%UiInstallationTarget{target: {:project_id, %OpaqueId{value: @installation}}}),
        navigation.(%UiInstallationTarget{
          target: {:repository_id, %OpaqueId{value: @installation}}
        })
      ]
    }

    stub = fn _channel, _request, _options -> {:ok, response} end

    assert {:ok, %{"installations" => [global, project, repository]}} =
             Client.list_ui_installations(identity(), @organization, :global, "",
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert global["target"] == %{"target_kind" => "global", "target_id" => nil}
    assert project["target"] == %{"target_kind" => "project", "target_id" => @installation}
    assert repository["target"] == %{"target_kind" => "repository", "target_id" => @installation}
    assert global["lifecycle"] == "enabled"
    assert global["icon"] == "app"
    assert global["presentation"] == "iframe"
    assert global["content_kind"] == "static"
    assert global["launchable"]
  end

  test "install sends explicit target and projects enabled lifecycle" do
    caller = self()

    stub = fn _channel, request, options ->
      send(caller, {:install_call, request, options})

      {:ok,
       %InstallUiResponse{
         installation_id: %OpaqueId{value: @installation},
         generation_id: %OpaqueId{value: @generation},
         lifecycle: UiInstallationLifecycle.value(:UI_INSTALLATION_LIFECYCLE_ENABLED)
       }}
    end

    assert {:ok,
            %{
              "installation_id" => @installation,
              "generation_id" => @generation,
              "lifecycle" => "enabled"
            }} =
             Client.install_ui(
               identity(),
               @organization,
               {:repository, @installation},
               "70000000-0000-4000-8000-000000000007",
               "reference-session-chat",
               idempotency_key: "attempt-1:install_ui",
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert_receive {:install_call, request, options}
    assert request.organization_id.value == @organization
    assert request.target.target == {:repository_id, %OpaqueId{value: @installation}}
    assert request.release_id.value == "70000000-0000-4000-8000-000000000007"
    assert request.ui_key == "reference-session-chat"
    assert request.acknowledge_repository_git_access == false
    assert request.context.idempotency_key == "attempt-1:install_ui"
    assert options[:metadata]["x-request-id"] =~ ~r/\A[0-9a-f-]{36}\z/i
  end

  test "install forwards repository Git acknowledgement only when explicitly enabled" do
    caller = self()

    stub = fn _channel, request, _options ->
      send(caller, {:acknowledgement, request.acknowledge_repository_git_access})
      {:ok, %InstallUiResponse{}}
    end

    assert {:ok, _} =
             Client.install_ui(
               identity(),
               @organization,
               {:repository, @installation},
               @generation,
               "session-chat",
               acknowledge_repository_git_access: true,
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert_receive {:acknowledgement, true}
  end

  test "install rejects an unsupported target before making an RPC" do
    assert {:error, %HephaestusWeb.RPC.Error{kind: :invalid}} =
             Client.install_ui(
               identity(),
               @organization,
               {:organization, @organization},
               @installation,
               "reference-session-chat"
             )
  end

  test "handoff sends fresh request context, empty idempotency, exact secret, and no retry" do
    caller = self()

    stub = fn _channel, request, options ->
      send(caller, {:handoff_call, request, options})

      {:ok,
       %CreateUiBrowserHandoffResponse{
         handoff_id: %OpaqueId{value: "40000000-0000-4000-8000-000000000004"},
         installation_id: %OpaqueId{value: @installation},
         generation_id: %OpaqueId{value: @generation},
         route: "docs"
       }}
    end

    assert {:ok, %{"handoff_id" => "40000000-0000-4000-8000-000000000004"}} =
             Client.create_ui_browser_handoff(
               identity(),
               @installation,
               @generation,
               "docs",
               stub_call: stub,
               channel_provider: channel_provider()
             )

    assert_receive {:handoff_call, request, options}
    assert request.context.idempotency_key == ""
    assert %OpaqueId{value: request_id} = request.context.request_id
    assert String.match?(request_id, ~r/\A[0-9a-f-]{36}\z/i)
    assert byte_size(request.handoff_secret) == 32
    assert options[:metadata]["x-request-id"] == request_id
    assert options[:retry] == nil
  end

  test "handoff callback receives the bearer only on success and default projection omits it" do
    caller = self()

    stub = fn _channel, request, _options ->
      send(caller, {:secret, request.handoff_secret})
      {:ok, %CreateUiBrowserHandoffResponse{}}
    end

    assert {:ok, safe_response} =
             Client.create_ui_browser_handoff(
               identity(),
               @installation,
               @generation,
               "docs",
               stub_call: stub,
               channel_provider: channel_provider(),
               on_success: fn safe_response, secret ->
                 send(caller, {:callback, safe_response, secret})
                 {:ok, safe_response}
               end
             )

    assert Map.has_key?(safe_response, "handoff_id")
    refute Map.has_key?(safe_response, "handoff_secret")
    assert_receive {:secret, secret}
    assert_receive {:callback, ^safe_response, ^secret}
  end

  test "handoff wire projection preserves route for the browser route-base adapter" do
    response = %CreateUiBrowserHandoffResponse{
      handoff_id: %OpaqueId{value: "40000000-0000-4000-8000-000000000004"},
      installation_id: %OpaqueId{value: @installation},
      generation_id: %OpaqueId{value: @generation},
      route: "docs"
    }

    wire = response |> CreateUiBrowserHandoffResponse.encode() |> IO.iodata_to_binary()
    decoded = CreateUiBrowserHandoffResponse.decode(wire)

    stub = fn _channel, _request, _options -> {:ok, decoded} end

    assert {:ok, url} =
             Client.create_ui_browser_handoff(
               identity(),
               @installation,
               @generation,
               "docs",
               stub_call: stub,
               channel_provider: channel_provider(),
               on_success: fn handoff, secret ->
                 assert handoff["route"] == "docs"

                 handoff
                 |> Map.put("route_base", handoff["route"])
                 |> UIBrowser.bootstrap_url(secret, "light",
                   namespace: "ui.example.com",
                   platform_host: "example.com"
                 )
               end
             )

    assert String.starts_with?(
             url,
             "https://g-#{String.replace(@generation, "-", "")}.ui.example.com/"
           )

    assert url =~ "/_heph/bootstrap?heph_theme=light#"
  end

  test "handoff does not retry an unavailable mutation or invoke its callback" do
    caller = self()

    stub = fn _channel, _request, _options ->
      send(caller, :handoff_attempt)
      {:error, GRPC.RPCError.exception(status: :unavailable)}
    end

    assert {:error, %HephaestusWeb.RPC.Error{kind: :unavailable}} =
             Client.create_ui_browser_handoff(
               identity(),
               @installation,
               @generation,
               "docs",
               stub_call: stub,
               channel_provider: channel_provider(),
               channel_reset: fn -> send(caller, :channel_reset) end,
               on_success: fn _response, _secret -> flunk("callback must not run") end
             )

    assert_receive :handoff_attempt
    assert_receive :channel_reset
    refute_receive :handoff_attempt
  end

  defp identity do
    %Identity{
      user_id: "50000000-0000-4000-8000-000000000005",
      issuer: "https://issuer.example",
      subject: "subject",
      display_name: "Test User",
      sid: "60000000-0000-4000-8000-000000000006"
    }
  end

  defp channel_provider, do: fn -> {:ok, %GRPC.Channel{}} end
end
