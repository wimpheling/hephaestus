defmodule Fixture.DesignSystem do
  alias Fixture.DesignSystem.Components.Button

  defdelegate button(assigns), to: Button

  def catalog do
    [
      %{
        name: :button,
        tier: :component,
        module: Button,
        function: :button,
        attrs: [],
        slots: [],
        showcase_id: "button",
        a11y_test_id: "button-accessible-name"
      }
    ]
  end
end
