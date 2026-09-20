defmodule HephaestusWebWeb.InstalledUiNavigationTest do
  use ExUnit.Case, async: true

  use HephaestusWebWeb, :html

  import Phoenix.LiveViewTest

  alias HephaestusWeb.RPC.Error
  alias HephaestusWebWeb.DesignSystem.Composites.InstalledUiNavigation
  alias HephaestusWebWeb.InstalledUiNavigationState

  @installation %{
    "installation_id" => "installation-1",
    "generation_id" => "generation-1",
    "ui_key" => "assistant",
    "label" => "Assistant",
    "icon" => "chat",
    "presentation" => "iframe",
    "lifecycle" => "enabled",
    "launchable" => true,
    "route_base" => "docs"
  }

  test "renders safe launch metadata and isolated browser host" do
    html =
      render_component(&InstalledUiNavigation.installed_ui_navigation/1,
        scope: :project,
        state: :ready,
        installations: [@installation]
      )

    document = LazyHTML.from_fragment(html)

    assert count(document, "#installed-ui-navigation-project") == 1
    assert count(document, "[data-ui-installation]") == 1
    assert count(document, "button[phx-value-id=installation-1]") == 1
    assert count(document, "iframe[sandbox='allow-scripts allow-same-origin']") == 1
    assert count(document, "iframe[referrerpolicy='no-referrer']") == 1
    refute html =~ "generation-1"
    refute html =~ "docs"
    refute html =~ "secret"
  end

  test "renders unavailable and terminal states without a launch control" do
    for state <- [:access_revoked, :terminated, :error] do
      html =
        render_component(&InstalledUiNavigation.installed_ui_navigation/1,
          scope: :organization,
          state: state,
          error: "Unavailable"
        )

      assert html =~ "Unavailable"
      refute html =~ "phx-value-id"
    end
  end

  test "uses the same safe host contract for organization, project, and repository shells" do
    for scope <- [:organization, :project, :repository] do
      html =
        render_component(&InstalledUiNavigation.installed_ui_navigation/1,
          scope: scope,
          state: :ready,
          installations: []
        )

      assert html =~ "installed-ui-navigation-#{scope}"
      assert html =~ "sandbox=\"allow-scripts allow-same-origin\""
      refute html =~ "http://"
    end
  end

  test "renders a bounded load-more action when another page is available" do
    html =
      render_component(&InstalledUiNavigation.installed_ui_navigation/1,
        scope: :organization,
        state: :ready,
        has_more: true
      )

    assert html =~ "Load more"
    assert html =~ "load-more-installed-ui"
  end

  test "filters removed entries and requires a complete current launch projection" do
    state = InstalledUiNavigationState.new("org-1", :global)

    state =
      InstalledUiNavigationState.complete(state, {
        :ok,
        %{"installations" => [@installation, %{@installation | "lifecycle" => "removed"}]}
      })

    assert state.status == :ready
    assert state.installations == [@installation]
    assert {:ok, @installation} = InstalledUiNavigationState.launch_entry(state, "installation-1")
    assert {:error, :unavailable} = InstalledUiNavigationState.launch_entry(state, "missing")
  end

  test "revoked authority clears the projection and prevents launch" do
    state = InstalledUiNavigationState.new("org-1", {:project, "project-1"})

    state =
      InstalledUiNavigationState.complete(
        state,
        {:error, %Error{kind: :permission_denied, retryable: false}}
      )

    assert state.status == :access_revoked
    assert state.installations == []

    assert {:error, :access_revoked} =
             InstalledUiNavigationState.launch_entry(state, "installation-1")
  end

  test "clears the active browser entry when refresh changes or disables its generation" do
    state = InstalledUiNavigationState.new("org-1", :global)

    state =
      InstalledUiNavigationState.complete(state, {:ok, %{"installations" => [@installation]}})

    state = InstalledUiNavigationState.activate(state, @installation)

    changed_generation = %{@installation | "generation_id" => "generation-2"}

    refreshed =
      InstalledUiNavigationState.complete(
        state,
        {:ok, %{"installations" => [changed_generation]}}
      )

    assert refreshed.status == :ready
    assert refreshed.installations == [changed_generation]
    assert refreshed.active_installation_id == nil
    assert refreshed.active_generation_id == nil

    disabled = %{@installation | "lifecycle" => "disabled", "launchable" => false}

    disabled_state =
      InstalledUiNavigationState.complete(state, {:ok, %{"installations" => [disabled]}})

    assert disabled_state.active_installation_id == nil
    assert disabled_state.active_generation_id == nil
  end

  test "retains loaded pages while refreshing the first page and avoids false revocation" do
    second = %{@installation | "installation_id" => "installation-2"}
    state = InstalledUiNavigationState.new("org-1", :global)

    state =
      InstalledUiNavigationState.complete(state, {
        :ok,
        %{
          "installations" => [@installation],
          "page" => %{"next_page_token" => "page-2"}
        }
      })

    assert InstalledUiNavigationState.present(state).has_more

    state =
      state
      |> InstalledUiNavigationState.load_more()
      |> then(fn state ->
        InstalledUiNavigationState.complete(state, {
          :ok,
          %{"installations" => [second], "page" => %{"next_page_token" => ""}}
        })
      end)

    assert Enum.map(state.installations, & &1["installation_id"]) == [
             "installation-1",
             "installation-2"
           ]

    refreshed =
      state
      |> InstalledUiNavigationState.refresh()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [@installation], "page" => %{"next_page_token" => "page-2"}}
      })

    assert Enum.map(refreshed.installations, & &1["installation_id"]) == [
             "installation-1",
             "installation-2"
           ]

    refreshed = InstalledUiNavigationState.activate(refreshed, second)
    assert refreshed.active_installation_id == "installation-2"
    assert refreshed.active_generation_id == "generation-1"
  end

  test "refreshes the active second page and closes removed or replaced entries" do
    second = %{@installation | "installation_id" => "installation-2"}
    state = InstalledUiNavigationState.new("org-1", :global)

    state =
      state
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [@installation], "page" => %{"next_page_token" => "page-2"}}
      })
      |> InstalledUiNavigationState.load_more()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [second], "page" => %{"next_page_token" => "page-3"}}
      })
      |> InstalledUiNavigationState.activate(second)

    refreshed =
      state
      |> InstalledUiNavigationState.refresh()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{
          "installations" => [%{second | "generation_id" => "generation-2"}],
          "page" => %{"next_page_token" => "page-3"}
        }
      })

    assert refreshed.active_installation_id == nil
    assert refreshed.active_generation_id == nil

    assert Enum.find(refreshed.installations, &(&1["installation_id"] == "installation-2"))[
             "generation_id"
           ] ==
             "generation-2"

    refreshed =
      InstalledUiNavigationState.activate(
        refreshed,
        %{second | "generation_id" => "generation-2"}
      )

    removed =
      refreshed
      |> InstalledUiNavigationState.refresh()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [], "page" => %{"next_page_token" => "page-3"}}
      })

    assert removed.active_installation_id == nil
    refute Enum.any?(removed.installations, &(&1["installation_id"] == "installation-2"))
  end

  test "continues from the last loaded cursor after refreshing the first page" do
    second = %{@installation | "installation_id" => "installation-2"}
    third = %{@installation | "installation_id" => "installation-3"}
    state = InstalledUiNavigationState.new("org-1", :global)

    state =
      state
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [@installation], "page" => %{"next_page_token" => "page-2"}}
      })
      |> InstalledUiNavigationState.load_more()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [second], "page" => %{"next_page_token" => "page-3"}}
      })

    state =
      state
      |> InstalledUiNavigationState.refresh()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [@installation], "page" => %{"next_page_token" => "page-2"}}
      })

    assert state.next_page_token == "page-3"

    state =
      state
      |> InstalledUiNavigationState.load_more()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [third], "page" => %{"next_page_token" => ""}}
      })

    assert Enum.map(state.installations, & &1["installation_id"]) == [
             "installation-1",
             "installation-2",
             "installation-3"
           ]
  end

  test "terminating clears retained pagination state" do
    state = InstalledUiNavigationState.new("org-1", :global)

    state =
      InstalledUiNavigationState.complete(state, {
        :ok,
        %{"installations" => [@installation], "page" => %{"next_page_token" => "page-2"}}
      })

    terminated = InstalledUiNavigationState.terminate(state)
    assert terminated.pages == %{}
    assert terminated.page_order == []
    assert terminated.next_page_token == nil
    assert terminated.request_page_token == ""
  end

  test "resets pagination and active authority when a retained cursor is invalidated" do
    state = InstalledUiNavigationState.new("org-1", :global)

    state =
      state
      |> InstalledUiNavigationState.complete({
        :ok,
        %{"installations" => [@installation], "page" => %{"next_page_token" => "page-2"}}
      })
      |> InstalledUiNavigationState.load_more()
      |> InstalledUiNavigationState.complete({
        :ok,
        %{
          "installations" => [%{@installation | "installation_id" => "installation-2"}],
          "page" => %{}
        }
      })

    state =
      state
      |> then(fn state ->
        InstalledUiNavigationState.activate(
          state,
          Enum.find(state.installations, &(&1["installation_id"] == "installation-2"))
        )
      end)

    reset =
      state
      |> InstalledUiNavigationState.refresh()
      |> InstalledUiNavigationState.complete({
        :error,
        %Error{kind: :invalid, retryable: false}
      })

    assert reset.status == :error
    assert reset.installations == []
    assert reset.pages == %{}
    assert reset.active_installation_id == nil
    assert reset.next_page_token == nil
  end

  defp count(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.to_tree()
    |> length()
  end
end
