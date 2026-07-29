defmodule HephaestusWebWeb.ProjectComponents do
  @moduledoc """
  Project-local navigation shared by project resources.
  """

  use HephaestusWebWeb, :html

  attr :project_id, :string, required: true
  attr :active, :atom, required: true, values: [:repositories, :agents, :runs, :settings]

  def project_tabs(assigns) do
    ~H"""
    <nav id="project-tabs" class="repository-tabs" aria-label="Project">
      <.link
        navigate={~p"/projects/#{@project_id}"}
        class={["repository-tab", @active == :repositories && "active"]}
        aria-current={if(@active == :repositories, do: "page")}
      >
        <.icon name="hero-circle-stack" class="size-4" /> Repositories
      </.link>
      <.link
        navigate={~p"/projects/#{@project_id}/agents"}
        class={["repository-tab", @active == :agents && "active"]}
        aria-current={if(@active == :agents, do: "page")}
      >
        <.icon name="hero-cpu-chip" class="size-4" /> Agents
      </.link>
      <.link
        navigate={~p"/projects/#{@project_id}/runs"}
        class={["repository-tab", @active == :runs && "active"]}
        aria-current={if(@active == :runs, do: "page")}
      >
        <.icon name="hero-play-circle" class="size-4" /> Runs
      </.link>
      <.link
        navigate={~p"/projects/#{@project_id}/settings"}
        class={["repository-tab", @active == :settings && "active"]}
        aria-current={if(@active == :settings, do: "page")}
      >
        <.icon name="hero-cog-6-tooth" class="size-4" /> Settings
      </.link>
    </nav>
    """
  end
end
