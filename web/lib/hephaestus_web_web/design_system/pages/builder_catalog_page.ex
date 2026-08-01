defmodule HephaestusWebWeb.DesignSystem.Pages.BuilderCatalogPage do
  @moduledoc "Pure presentation for the platform-owned builder image catalog."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :builder_images, :list, default: []
  attr :item_count, :integer, default: 0
  attr :error, :any, default: nil

  @doc "Renders catalog identity, policy, provenance, and availability metadata."
  def builder_catalog_page(assigns) do
    ~H"""
    <.page_state
      id="builder-catalog-page-state"
      state={@state}
      title="Builder catalog unavailable"
      message={@error || "The platform builder catalog is not ready."}
    >
      <.frame variant={:summary_body}>
        <.page_heading
          eyebrow="Platform-owned build environments"
          title="Builder image catalog"
          description="Only prepared, digest-pinned images approved by platform policy may be selected by agent.toml."
        >
          <:actions>
            <.tag>{@item_count} catalog entries</.tag>
          </:actions>
        </.page_heading>
        <.text :if={@builder_images == []} as="p" id="builder-catalog-empty" variant={:empty}>
          No builder images are currently available.
        </.text>
        <.frame
          :for={builder <- @builder_images}
          as="article"
          id={"builder-image-#{builder["id"]}"}
          variant={:proposal}
        >
          <.frame variant={:summary_body}>
            <.frame variant={:summary_body}>
              <.text as="strong">{builder["display_name"]}</.text>
              <.text as="span" variant={:muted}>{builder["key"]}</.text>
            </.frame>
            <.tag>{builder["availability"] || "unknown"}</.tag>
          </.frame>
          <.frame as="dl" variant={:review_grid}>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Immutable reference</.text>
              <.text as="dd" variant={:mono}>{builder["image_reference"]}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Preparation</.text>
              <.text as="dd">{builder["preparation"] || "unknown"}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Architectures</.text>
              <.text as="dd">{Enum.join(builder["architectures"] || [], ", ")}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Build network ceiling</.text>
              <.text as="dd">{network_label(builder["network_ceiling"])}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Resource ceiling</.text>
              <.text as="dd">
                {builder["max_vcpus"]} vCPU / {builder["max_memory_mib"]} MiB
              </.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Dependency policy</.text>
              <.text as="dd">{builder["dependency_policy"] || "unknown"}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Toolchains</.text>
              <.text as="dd">
                {toolchains(builder["toolchains"] || [])}
              </.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Provenance</.text>
              <.text as="dd">{get_in(builder, ["provenance", "source"])}</.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>Signature</.text>
              <.text as="dd" variant={:mono}>
                {get_in(builder, ["provenance", "signature"]) || "Not supplied"}
              </.text>
            </.frame>
            <.frame as="div" variant={:panel}>
              <.text as="dt" variant={:muted}>SBOM</.text>
              <.text as="dd" variant={:mono}>
                {get_in(builder, ["provenance", "sbom"]) || "Not supplied"}
              </.text>
            </.frame>
          </.frame>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp network_label("disabled"), do: "Disabled"
  defp network_label("broker_only"), do: "Broker only"
  defp network_label("egress"), do: "Constrained egress"
  defp network_label(value), do: value || "Unknown"

  defp toolchains(toolchains) do
    Enum.map_join(toolchains, ", ", fn toolchain ->
      "#{toolchain["name"]} #{toolchain["version"]}"
    end)
  end
end
