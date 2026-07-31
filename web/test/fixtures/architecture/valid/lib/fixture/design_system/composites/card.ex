defmodule Fixture.DesignSystem.Composites.Card do
  use Phoenix.Component

  alias Fixture.DesignSystem

  attr :label, :string, required: true

  def card(assigns) do
    ~H"""
    <DesignSystem.button label={@label} />
    """
  end
end
