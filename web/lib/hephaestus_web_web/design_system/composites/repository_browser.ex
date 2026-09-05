defmodule HephaestusWebWeb.DesignSystem.Composites.RepositoryBrowser do
  @moduledoc "Repository navigation, tree, and viewer framing."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1]

  attr :id, :string, default: "repository-browser"
  slot :navigation
  slot :tree, required: true
  slot :content, required: true

  @doc "Renders the bounded repository-browser regions."
  def repository_browser(assigns) do
    ~H"""
    <.frame as="section" id={@id} variant={:repository_browser} aria_label="Repository browser">
      <.frame as="aside" variant={:file_browser} aria_label="Repository files">
        {render_slot(@navigation)}
        {render_slot(@tree)}
      </.frame>
      <.frame as="section" variant={:file_viewer} aria_label="File preview">
        {render_slot(@content)}
      </.frame>
    </.frame>
    """
  end
end
