defmodule HephaestusWebWeb.DesignSystem.Composites.RepositoryShell do
  @moduledoc "Shared repository identity, navigation, and lifecycle chrome."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem,
    only: [breadcrumbs: 1, frame: 1, glyph: 1, page_state: 1, tab_navigation: 1, tag: 1, text: 1]

  attr :state, :atom,
    required: true,
    values: [:loading, :error, :reconnecting, :ready]

  attr :repository, :map, default: nil
  attr :tabs, :list, required: true

  attr :active, :atom,
    required: true,
    values: [:files, :commits, :branches, :builds, :releases, :agents]

  attr :organization_index_destination, :string, default: nil
  attr :organization_destination, :string, default: nil
  attr :project_destination, :string, default: nil
  slot :inner_block, required: true

  @doc "Renders repository chrome and one route-specific page body."
  def repository_shell(assigns) do
    ~H"""
    <.page_state
      :if={@state != :ready}
      id="repository-page-state"
      state={@state}
      title="Repository unavailable"
      message="Resolving repository access and branch state."
    />
    <.frame :if={@state == :ready} variant={:summary_body}>
      <.breadcrumbs id="repository-breadcrumbs">
        <:item navigate={@organization_index_destination}>Organizations</:item>
        <:item navigate={@organization_destination}>{@repository["organization_name"]}</:item>
        <:item navigate={@project_destination}>{@repository["project_name"]}</:item>
        <:current>{@repository["name"]}</:current>
      </.breadcrumbs>

      <.frame as="section" variant={:organization_header}>
        <.frame variant={:organization_mark}>
          <.glyph name="hero-command-line" size={:large} />
        </.frame>
        <.frame variant={:organization_body}>
          <.text as="p" variant={:eyebrow}>Git repository</.text>
          <.text as="h1" variant={:title}>{@repository["name"]}</.text>
          <.text as="p" variant={:mono}>{friendly_ref(@repository["default_branch"])}</.text>
        </.frame>
        <.frame variant={:page_heading_actions}>
          <.tag tone={if(@repository["is_public"], do: "success", else: "neutral")}>
            {if(@repository["is_public"], do: "public", else: "private")}
          </.tag>
          <.tag tone="success" dot>live</.tag>
        </.frame>
      </.frame>

      <.tab_navigation id="repository-tabs" label="Repository" items={@tabs} active={@active} />
      {render_slot(@inner_block)}
    </.frame>
    """
  end

  defp friendly_ref("refs/heads/" <> branch), do: branch
  defp friendly_ref(git_ref), do: git_ref
end
