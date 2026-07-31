defmodule HephaestusWebWeb.DesignSystem.Pages.ReleasePageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.ReleasePage

  @covered_states [:loading, :error, :reconnecting, :ready]
  @status_visual_states %{
    initial: :loading,
    loading: :loading,
    ready: :ready,
    submitting: :loading,
    error: :error,
    stale: :reconnecting,
    reconnecting: :reconnecting,
    access_revoked: :error
  }

  test "renders every non-ready lifecycle state" do
    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)

    for state <- [:loading, :error, :reconnecting] do
      html = render_component(&ReleasePage.release/1, Map.put(assigns(), :state, state))

      assert html =~ ~s(id="release-page-state")
      assert html =~ "Release unavailable"
    end
  end

  test "renders release provenance and streamed contents from explicit destinations" do
    assert @covered_states == [:loading, :error, :reconnecting, :ready]

    html = render_component(&ReleasePage.release/1, assigns())

    assert html =~ ~s(id="release-breadcrumbs")
    assert html =~ ~s(id="release-provenance")
    assert html =~ ~s(id="release-artifacts")
    assert html =~ ~s(id="release-artifact-artifact-1")
    assert html =~ ~s(id="release-agents")
    assert html =~ ~s(href="/repositories/repository-1/commits?ref=refs%2Fheads%2Fmain")
  end

  defp assigns do
    %{
      state: :ready,
      release: %{
        "version" => "1.0.0",
        "state" => "published",
        "organization_name" => "Acme",
        "project_name" => "Forge",
        "repository_name" => "Source",
        "source_ref" => "refs/heads/main",
        "source_commit" => "1234567890abcdef",
        "build_request_id" => "build-request-1",
        "build_state" => "succeeded",
        "manifest_hash" => "manifest-hash",
        "configuration_hash" => "configuration-hash",
        "artifacts" => [artifact()],
        "agents" => [agent()]
      },
      artifacts: [{"release-artifact-artifact-1", artifact()}],
      agents: [{"release-agent-agent-1", agent()}],
      organization_index_destination: "/organizations",
      organization_destination: "/organizations/org-1",
      project_destination: "/projects/project-1",
      repository_releases_destination: "/repositories/repository-1/releases",
      source_destination: "/repositories/repository-1/commits?ref=refs%2Fheads%2Fmain"
    }
  end

  defp artifact do
    %{
      "id" => "artifact-1",
      "path" => "bin/cook",
      "media_type" => "application/octet-stream",
      "kind" => "runtime",
      "mode" => 493,
      "size_bytes" => 2_048,
      "content_hash" => "artifact-content-hash"
    }
  end

  defp agent do
    %{
      "id" => "agent-1",
      "display_name" => "Cook",
      "agent_key" => "cook",
      "requires_state" => true,
      "parameter_schema" => [],
      "secret_slot_schema" => []
    }
  end
end
