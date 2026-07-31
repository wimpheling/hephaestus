defmodule HephaestusWebWeb.DesignSystem.Composites.TabNavigation do
  @moduledoc "Navigation for a bounded set of peer product destinations."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [action: 1, frame: 1, glyph: 1]

  attr :id, :string, required: true
  attr :label, :string, required: true
  attr :items, :list, required: true
  attr :active, :atom, required: true

  @doc "Renders peer destinations with exactly one optional current item."
  def tab_navigation(assigns) do
    ~H"""
    <.frame as="nav" id={@id} variant={:tabs} aria_label={@label}>
      <.action
        :for={item <- @items}
        interaction={:navigate}
        destination={item.destination}
        variant={:tab}
        current={item.key == @active}
      >
        <.glyph :if={item[:icon]} name={item.icon} /> {item.label}
      </.action>
    </.frame>
    """
  end
end
