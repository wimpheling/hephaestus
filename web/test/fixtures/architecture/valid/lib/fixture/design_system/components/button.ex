defmodule Fixture.DesignSystem.Components.Button do
  use Phoenix.Component

  attr :label, :string, required: true

  def button(assigns) do
    ~H"""
    <button type="button">{@label}</button>
    """
  end
end
