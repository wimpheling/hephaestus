defmodule HephaestusWebWeb.RepositoryPageFixtures do
  @moduledoc false

  def model do
    repository = %{
      "id" => "repository-1",
      "name" => "Source",
      "organization_name" => "Acme",
      "project_name" => "Forge",
      "default_branch" => "refs/heads/main",
      "is_public" => false
    }

    %{
      state: :ready,
      repository: repository,
      selected_branch: nil,
      branch_options: [],
      browse_form: %{"branch" => ""},
      branches_empty?: true,
      commits_empty?: true,
      builds_empty?: true,
      builds_unavailable?: false,
      releases_empty?: true,
      attached_instances_empty?: true,
      tree: %{name: "", path: "", directories: [], files: [], file_count: 0},
      current_path: nil,
      file: nil,
      file_error: nil,
      tabs: [
        %{
          key: :files,
          label: "Files",
          icon: "hero-folder",
          destination: "/repositories/repository-1/files"
        },
        %{
          key: :commits,
          label: "Commits",
          icon: "hero-clock",
          destination: "/repositories/repository-1/commits"
        },
        %{
          key: :branches,
          label: "Branches",
          icon: "hero-code-bracket",
          destination: "/repositories/repository-1/branches"
        },
        %{
          key: :builds,
          label: "Builds",
          icon: "hero-cpu-chip",
          destination: "/repositories/repository-1/builds"
        },
        %{
          key: :releases,
          label: "Releases",
          icon: "hero-cube-transparent",
          destination: "/repositories/repository-1/releases"
        },
        %{
          key: :agents,
          label: "Agents",
          icon: "hero-cpu-chip",
          destination: "/repositories/repository-1/agents"
        }
      ],
      destinations: %{
        organization_index: "/organizations",
        organization: "/organizations/organization-1",
        project: "/projects/project-1"
      }
    }
  end
end
