defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectBuildersPage do
  @moduledoc "Pure presentation for repository-owned OCI builders in a project."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project_id, :string, required: true
  attr :builders, :list, default: []
  attr :item_count, :integer, default: 0
  attr :error, :any, default: nil

  @doc "Renders repository configuration discoveries and preparation state."
  def project_builders_page(assigns) do
    ~H"""
    <.page_state
      id="project-builders-page-state"
      state={@state}
      title="Project builders unavailable"
      message={@error || "Repository-owned builders are not ready."}
    >
      <.frame variant={:summary_body}>
        <.page_heading
          eyebrow="Project configuration"
          title="Project-owned builders"
          description="Repository-owned builders are discovered from committed heph.builders.toml and prepared from their exact source revision."
        >
          <:actions>
            <.tag>{@item_count} discovered builders</.tag>
          </:actions>
        </.page_heading>
        <.frame as="section" id="project-builder-list" variant={:summary_body}>
          <.text :if={@builders == []} as="p" id="project-builder-empty" variant={:empty}>
            No repository builders were discovered in committed configuration.
          </.text>
          <.frame
            :for={builder <- @builders}
            as="article"
            id={"project-builder-#{builder["id"]}"}
            variant={:proposal}
          >
            <.frame variant={:summary_body}>
              <.frame variant={:summary_body}>
                <.text as="strong">{builder["display_name"]}</.text>
                <.text as="span" variant={:muted}>{builder["key"]}</.text>
              </.frame>
              <.tag>{builder["status"] || "unknown"}</.tag>
            </.frame>
            <.frame as="dl" variant={:review_grid}>
              <.frame as="div" variant={:panel}>
                <.text as="dt" variant={:muted}>Dockerfile</.text>
                <.text as="dd" variant={:mono}>{builder["dockerfile_path"]}</.text>
              </.frame>
              <.frame as="div" variant={:panel}>
                <.text as="dt" variant={:muted}>Prepared digest</.text>
                <.text as="dd" variant={:mono}>{builder["oci_image_digest"] || "Not prepared"}</.text>
              </.frame>
              <.frame as="div" variant={:panel}>
                <.text as="dt" variant={:muted}>Registry publication</.text>
                <.text as="dd">{registry_value(builder, "state")}</.text>
              </.frame>
              <.frame as="div" variant={:panel}>
                <.text as="dt" variant={:muted}>Registry availability</.text>
                <.text as="dd">{registry_value(builder, "availability")}</.text>
              </.frame>
              <.frame as="div" variant={:panel}>
                <.text as="dt" variant={:muted}>Registry digest</.text>
                <.text as="dd" variant={:mono}>
                  {registry_value(builder, "immutable_reference")}
                </.text>
              </.frame>
              <.frame as="div" variant={:panel}>
                <.text as="dt" variant={:muted}>Verified architectures</.text>
                <.text as="dd">
                  {Enum.join(get_in(builder, ["registry_publication", "architectures"]) || [], ", ")}
                </.text>
              </.frame>
              <.frame
                :for={kind <- ["sbom", "provenance", "scan", "signature"]}
                as="div"
                variant={:panel}
              >
                <.text as="dt" variant={:muted}>{String.upcase(kind)} verification</.text>
                <.text as="dd" variant={:mono}>{registry_evidence(builder, kind)}</.text>
              </.frame>
            </.frame>
          </.frame>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp registry_value(builder, key) do
    get_in(builder, ["registry_publication", key]) || "Not requested"
  end

  defp registry_evidence(builder, kind) do
    evidence = get_in(builder, ["registry_publication", kind]) || %{}
    evidence["immutable_reference"] || evidence["state"] || "Pending"
  end
end
