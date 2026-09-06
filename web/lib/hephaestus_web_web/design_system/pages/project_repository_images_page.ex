defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagesPage do
  @moduledoc "Pure presentation for redacted project repository OCI images."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project_id, :string, required: true
  attr :images, :list, default: []
  attr :image_destination, :any, required: true
  attr :retry_event, :string, default: nil

  def project_repository_images_page(assigns) do
    ~H"""
    <.page_state
      id="project-images-page-state"
      state={@state}
      title="Images unavailable"
      message="Project image resources are not ready."
    >
      <.frame variant={:summary_body}>
        <.page_heading
          eyebrow="Project artifacts"
          title="Images"
          description="Repository-owned OCI images are prepared from exact commits using approved bases."
        />
        <.tab_navigation id="project-tabs" label="Project" active={:images} items={tabs(@project_id)} />
        <.frame as="section" id="project-images" variant={:table}>
          <.resource_list id="project-image-resources" layout={:projects}>
            <:header>
              <.text as="span" variant={:muted}>Image</.text><.text as="span" variant={:muted}>
                Source
              </.text><.text as="span" variant={:muted}>Status</.text>
            </:header>
            <:empty>No repository image definitions are visible for this project.</:empty>
            <.frame
              :for={image <- @images}
              as="article"
              id={"project-image-#{image["id"]}"}
              variant={:resource_row}
            >
              <.text as="strong">{image["display_name"] || image["key"]}</.text>
              <.text as="code" variant={:mono}>{image["source_revision"]}</.text>
              <.frame variant={:resource_controls}>
                <.tag>{image["status"]}</.tag>
                <.action destination={@image_destination.(image["id"])} variant={:secondary}>
                  Details
                </.action>
                <.action
                  :if={@retry_event && image["status"] == "failed"}
                  interaction={:event}
                  event={@retry_event}
                  value={image["id"]}
                  variant={:secondary}
                  disable_with="Queueing image retry…"
                >
                  Retry image
                </.action>
              </.frame>
            </.frame>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp tabs(project_id),
    do: [
      %{
        key: :repositories,
        label: "Repositories",
        icon: "hero-circle-stack",
        destination: "/projects/#{project_id}"
      },
      %{
        key: :agents,
        label: "Agents",
        icon: "hero-cpu-chip",
        destination: "/projects/#{project_id}/agents"
      },
      %{
        key: :runs,
        label: "Runs",
        icon: "hero-play-circle",
        destination: "/projects/#{project_id}/runs"
      },
      %{
        key: :images,
        label: "Images",
        icon: "hero-cube",
        destination: "/projects/#{project_id}/images"
      },
      %{
        key: :gateways,
        label: "Gateways",
        icon: "hero-globe-alt",
        destination: "/projects/#{project_id}/gateways"
      },
      %{
        key: :settings,
        label: "Settings",
        icon: "hero-cog-6-tooth",
        destination: "/projects/#{project_id}/settings"
      }
    ]
end
