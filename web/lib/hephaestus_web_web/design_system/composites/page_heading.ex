defmodule HephaestusWebWeb.DesignSystem.Composites.PageHeading do
  @moduledoc "Consistent title, supporting copy, and page actions."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1, text: 1]

  attr :id, :string, default: nil
  attr :eyebrow, :string, required: true
  attr :title, :string, required: true
  attr :description, :string, default: nil
  attr :level, :string, default: "h1", values: ["h1", "h2", "h3"]
  slot :actions

  @doc "Renders a page or section heading with bounded hierarchy."
  def page_heading(assigns) do
    ~H"""
    <.frame as="section" id={@id} variant={:page_heading}>
      <.frame variant={:page_heading_copy}>
        <.text as="p" variant={:eyebrow}>{@eyebrow}</.text>
        <.text as={@level} variant={:title}>{@title}</.text>
        <.text :if={@description} as="p" variant={:lede}>{@description}</.text>
      </.frame>
      <.frame :if={@actions != []} variant={:page_heading_actions}>
        {render_slot(@actions)}
      </.frame>
    </.frame>
    """
  end
end
