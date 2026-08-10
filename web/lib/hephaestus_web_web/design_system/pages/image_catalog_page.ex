defmodule HephaestusWebWeb.DesignSystem.Pages.ImageCatalogPage do
  @moduledoc "Pure presentation for the platform-owned OCI image catalog."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :images, :list, default: []
  attr :item_count, :integer, default: 0
  attr :error, :any, default: nil

  @doc "Renders catalog identity, policy, provenance, and availability metadata."
  def image_catalog_page(assigns) do
    ~H"""
    <.page_state
      id="image-catalog-page-state"
      state={@state}
      title="Image catalog unavailable"
      message={@error || "The platform OCI image catalog is not ready."}
    >
      <.frame variant={:summary_body}>
        <.page_heading
          eyebrow="Platform-owned OCI images"
          title="Images"
          description="Prepared, digest-pinned OCI images approved by platform policy are available to execution contracts."
        >
          <:actions>
            <.tag>{@item_count} catalog entries</.tag>
          </:actions>
        </.page_heading>
        <.text :if={@images == []} as="p" id="image-catalog-empty" variant={:empty}>
          No OCI images are currently available.
        </.text>
        <.frame
          :for={image <- @images}
          as="article"
          id={"oci-image-#{image["id"]}"}
          variant={:proposal}
        >
          <.frame variant={:summary_body}>
            <.frame variant={:summary_body}>
              <.text as="strong">{image["display_name"]}</.text>
              <.text as="span" variant={:muted}>{image["key"]}</.text>
            </.frame>
            <.tag>{image["availability"] || "unknown"}</.tag>
          </.frame>
          <.frame as="dl" variant={:review_grid}>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Immutable reference</.text>
              <.text as="dd" variant={:mono}>{image["image_reference"]}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Preparation</.text>
              <.text as="dd">{image["preparation"] || "unknown"}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Architectures</.text>
              <.text as="dd">{Enum.join(image["architectures"] || [], ", ")}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Toolchains</.text>
              <.text as="dd">
                {toolchains(image["toolchains"] || [])}
              </.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Provenance</.text>
              <.text as="dd">{get_in(image, ["provenance", "source"])}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Signature</.text>
              <.text as="dd" variant={:mono}>
                {get_in(image, ["provenance", "signature"]) || "Not supplied"}
              </.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>SBOM</.text>
              <.text as="dd" variant={:mono}>
                {get_in(image, ["provenance", "sbom"]) || "Not supplied"}
              </.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Registry publication</.text>
              <.text as="dd">{registry_value(image, "state")}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Registry availability</.text>
              <.text as="dd">{registry_value(image, "availability")}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Registry digest</.text>
              <.text as="dd" variant={:mono}>
                {registry_value(image, "immutable_reference")}
              </.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Verified registry architectures</.text>
              <.text as="dd">
                {Enum.join(get_in(image, ["registry_publication", "architectures"]) || [], ", ")}
              </.text>
            </.frame>
            <.frame
              :for={kind <- ["sbom", "provenance", "scan", "signature"]}
              as="div"
              variant={:panel}
            >
              <.text as="dt" variant={:muted}>{String.upcase(kind)} verification</.text>
              <.text as="dd" variant={:mono}>
                {registry_evidence(image, kind)}
              </.text>
            </.frame>
          </.frame>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp toolchains(toolchains) do
    Enum.map_join(toolchains, ", ", fn toolchain ->
      "#{toolchain["name"]} #{toolchain["version"]}"
    end)
  end

  defp registry_value(image, key) do
    get_in(image, ["registry_publication", key]) || "Not requested"
  end

  defp registry_evidence(image, kind) do
    evidence = get_in(image, ["registry_publication", kind]) || %{}
    evidence["immutable_reference"] || evidence["state"] || "Pending"
  end
end
