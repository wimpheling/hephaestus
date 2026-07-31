defmodule HephaestusWebWeb.DesignSystem.Components.Form do
  @moduledoc "Bounded form boundary for pages and composites."

  use Phoenix.Component

  attr :for, :any, required: true
  attr :as, :atom, default: nil
  attr :id, :string, required: true
  attr :change, :string, default: nil
  attr :submit, :string, default: nil
  attr :layout, :atom, default: :default, values: [:default, :inline]
  slot :inner_block, required: true

  @doc "Renders a Phoenix form without exposing raw form construction above the basic tier."
  def form_container(assigns) do
    ~H"""
    <Phoenix.Component.form
      :let={form}
      for={@for}
      as={@as}
      id={@id}
      phx-change={@change}
      phx-submit={@submit}
      class={form_class(@layout)}
    >
      {render_slot(@inner_block, form)}
    </Phoenix.Component.form>
    """
  end

  defp form_class(:default), do: nil
  defp form_class(:inline), do: "resource-inline-form"
end
