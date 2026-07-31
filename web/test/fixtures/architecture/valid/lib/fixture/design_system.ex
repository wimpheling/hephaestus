defmodule Fixture.DesignSystem do
  alias Fixture.DesignSystem.Components.Button
  alias Fixture.DesignSystem.Composites.Card

  defdelegate button(assigns), to: Button
  defdelegate card(assigns), to: Card

  def catalog do
    [
      %{
        name: :button,
        tier: :component,
        module: Button,
        function: :button,
        attrs: [:label],
        slots: [],
        showcase_id: :button,
        a11y_test_id: :button
      },
      %{
        name: :card,
        tier: :composite,
        module: Card,
        function: :card,
        attrs: [:label],
        slots: [],
        showcase_id: :card,
        a11y_test_id: :card
      }
    ]
  end
end
