defmodule Fixture.DesignSystem.Composites.Panel do
  use Phoenix.Component

  alias Fixture.DesignSystem.Components.Button
  alias HephaestusWeb.Store

  attr :class, :any, default: nil
  attr :error_class, :any, default: nil
  attr :rest, :global
  attr :style, :string, default: nil
  attr :columns, :string, required: true

  def panel(assigns), do: ~H"<Button.button label={@label} />"

  def backend_reference, do: Store
end
