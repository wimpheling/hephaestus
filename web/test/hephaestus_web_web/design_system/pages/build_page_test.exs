defmodule HephaestusWebWeb.DesignSystem.Pages.BuildPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.BuildPage

  @covered_states [:loading, :error, :reconnecting, :ready]
  @covered_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  test "renders all supported build data and labels unsupported actions distinctly" do
    html = render_component(&BuildPage.build/1, assigns())

    for id <-
          ~w(build-breadcrumbs build-provenance build-metrics build-logs build-declaration
             build-timeline build-artifacts build-manifest build-release build-actions) do
      assert html =~ ~s(id="#{id}")
    end

    assert html =~ "commit-1"
    assert html =~ "a bounded log line"
    assert html =~ "push"
    assert html =~ "configuration-hash"
    assert html =~ "declared/output"
    assert html =~ "produced/output"
    assert html =~ "draft_release_created"
    assert html =~ "Retry attempt"
    assert html =~ "Rebuild for verification"
    assert html =~ "Build another commit"
    refute html =~ "Rebuild</"
  end

  test "renders loading, error, and reconnecting states" do
    assert length(@covered_states) == 4

    for state <- @covered_states do
      html = render_component(&BuildPage.build/1, Map.put(assigns(), :state, state))

      if state == :ready do
        assert html =~ ~s(id="build-provenance")
      else
        assert html =~ ~s(id="build-page-state")
      end
    end

    assert length(@covered_statuses) == 8
  end

  defp assigns do
    %{
      state: :ready,
      repository: %{
        "id" => "repository-1",
        "name" => "Source",
        "organization_name" => "Acme",
        "project_name" => "Project"
      },
      build: %{
        "id" => "build-1234567890",
        "state" => "succeeded",
        "exit_code" => 0,
        "failure_code" => "",
        "source_commit" => "commit-1",
        "created_at" => ~U[2026-08-01 10:00:00Z],
        "updated_at" => ~U[2026-08-01 10:01:00Z],
        "trigger" => "push",
        "agent_key" => "browser-reviewer",
        "builder_image_key" => "rust",
        "builder_image_reference" => "registry.example/rust@sha256:abc",
        "configuration_hash" => "configuration-hash",
        "duration_milliseconds" => 60_000,
        "parsed_declaration" => %{"command" => "build"},
        "build_policy" => %{"network" => "disabled"},
        "timeline" => [
          %{
            "to_state" => "succeeded",
            "reason" => "draft_release_created",
            "occurred_at" => ~U[2026-08-01 10:01:00Z]
          }
        ],
        "declared_artifacts" => [%{"path" => "declared/output", "kind" => "file"}],
        "produced_artifacts" => [
          %{"path" => "produced/output", "sha256" => "hash", "size_bytes" => 12}
        ],
        "artifact_manifest" => [%{"path" => "produced/output"}]
      },
      logs: ["a bounded log line"],
      metrics: [%{"name" => "duration", "value" => 1.0, "unit" => "seconds"}],
      timeline: [
        %{
          "to_state" => "succeeded",
          "reason" => "draft_release_created",
          "occurred_at" => ~U[2026-08-01 10:01:00Z]
        }
      ],
      declared_artifacts: [%{"path" => "declared/output", "kind" => "file"}],
      produced_artifacts: [%{"path" => "produced/output", "sha256" => "hash", "size_bytes" => 12}],
      artifact_manifest: [%{"path" => "produced/output"}],
      organization_index_destination: "/organizations",
      organization_destination: "/organizations/organization-1",
      project_destination: "/projects/project-1",
      repository_destination: "/repositories/repository-1/builds",
      release_destination: nil,
      retry_event: "retry-build",
      verification_rebuild_event: "verification-rebuild"
    }
  end
end
