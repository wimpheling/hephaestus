defmodule HephaestusWebWeb.DesignSystem.Pages.RunPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.RunPage

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
      html = render_component(&RunPage.run/1, Map.put(assigns(), :state, state))

      assert html =~ ~s(id="run-page-state")
      assert html =~ "Run unavailable"
    end
  end

  test "renders exact provenance, controls, review actions, timeline, and artifacts" do
    assert @covered_states == [:loading, :error, :reconnecting, :ready]

    html = render_component(&RunPage.run/1, assigns())

    for id <-
          ~w(run-breadcrumbs run-exact-provenance runtime-metrics result-diff run-timeline artifacts) do
      assert html =~ ~s(id="#{id}")
    end

    for test_id <-
          ~w(cancel-run retry-run review-proposal reject-result approve-result result-diff run-timeline) do
      assert html =~ ~s(data-testid="#{test_id}")
    end

    assert html =~ ~s(phx-click="control")
    assert html =~ ~s(phx-value-kind="cancel_run")
    assert html =~ ~s(phx-value-kind="approve_result")
    assert html =~ ~s(href="/releases/release-1")
    assert html =~ ~s(href="/agents/agent-1")
  end

  defp assigns do
    run = %{
      "id" => "run-1234567890",
      "organization_name" => "Acme",
      "repository_name" => "Source",
      "attempt" => 1,
      "agent_name" => "Cook",
      "state" => "running",
      "outcome" => nil,
      "input_commit" => "1234567890abcdef",
      "git_ref" => "refs/heads/main",
      "release_version" => "1.0.0",
      "instance_revision_id" => "revision-1234567890",
      "metrics" => %{"elapsed_ms" => 50, "event_count" => 1, "log_count" => 1},
      "runtime_metrics" => [%{"name" => "cpu", "value" => 0.5, "labels" => %{}}],
      "proposal_id" => "proposal-1",
      "proposal_state" => "open",
      "target_ref" => "refs/heads/main",
      "result_commit" => "abcdef1234567890",
      "result_message" => "Ready",
      "artifact_manifest_hash" => "manifest-1234567890",
      "artifacts" => [%{"kind" => "patch", "size_bytes" => 10}]
    }

    %{
      state: :ready,
      run: run,
      patch: "diff --git",
      manifest: "{}",
      events: [
        {"event-1",
         %{
           "sequence" => 1,
           "event_type" => "vm.log",
           "occurred_at" => ~U[2026-07-31 10:00:00Z],
           "payload" => %{"line" => "running"}
         }}
      ],
      artifacts: [
        {"artifact-1",
         %{
           "kind" => "patch",
           "path" => "result.patch",
           "size_bytes" => 10,
           "sha256" => "abcdef1234567890"
         }}
      ],
      organization_index_destination: "/organizations",
      organization_destination: "/organizations/org-1",
      repository_destination: "/repositories/repository-1",
      release_destination: "/releases/release-1",
      agent_destination: "/agents/agent-1",
      control_event: "control"
    }
  end
end
