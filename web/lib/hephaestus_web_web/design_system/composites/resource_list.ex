defmodule HephaestusWebWeb.DesignSystem.Composites.ResourceList do
  @moduledoc "Accessible framed collections with bounded column layouts."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1]

  attr :id, :string, required: true

  attr :layout, :atom,
    default: :default,
    values: [:default, :compact, :projects, :secrets, :grants]

  attr :update, :string, default: nil, values: [nil, "stream"]
  attr :aria_label, :string, default: "Resources"
  slot :header, required: true
  slot :row
  slot :empty
  slot :inner_block

  @doc "Renders a resource-list frame while callers provide component-only rows."
  def resource_list(assigns) do
    ~H"""
    <.frame
      as="section"
      id={@id}
      variant={:resource_list}
      layout={@layout}
      phx_update={@update}
      aria_label={@aria_label}
    >
      <.frame id={"resource-heading-#{@id}"} variant={:resource_heading}>
        {render_slot(@header)}
      </.frame>
      <.frame :if={@empty != []} id={"resource-empty-#{@id}"} variant={:resource_empty}>
        {render_slot(@empty)}
      </.frame>
      {render_slot(@row)}
      {render_slot(@inner_block)}
    </.frame>
    """
  end
end
