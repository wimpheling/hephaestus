defmodule HephaestusWebWeb.DesignSystem.Components.Navigation do
  @moduledoc """
  Implements the basic navigation and metadata primitives.

  Feature-specific navigation and domain components belong with their feature,
  not in this module.
  """

  use Phoenix.Component

  @doc """
  Renders hierarchy-based navigation.

  Every `item` is an ancestor and therefore requires a destination. The
  `current` slot is rendered as plain text and marked for assistive technology.
  """
  slot :item, required: true do
    attr :navigate, :string, required: true
  end

  slot :current, required: true

  attr :id, :string, default: nil

  def breadcrumbs(assigns) do
    ~H"""
    <nav id={@id} class="breadcrumbs" aria-label="Breadcrumb">
      <ol>
        <li :for={item <- @item}>
          <.link navigate={item.navigate}>{render_slot(item)}</.link>
          <span class="breadcrumb-separator" aria-hidden="true">/</span>
        </li>
        <li class="breadcrumb-current">
          <span aria-current="page">{render_slot(@current)}</span>
        </li>
      </ol>
    </nav>
    """
  end

  @doc """
  Renders compact, reusable metadata.
  """
  attr :tone, :string,
    default: "neutral",
    values: ~w(neutral accent success warning danger)

  attr :dot, :boolean, default: false
  attr :variant, :atom, default: :default, values: [:default, :environment]
  slot :inner_block, required: true

  def tag(assigns) do
    ~H"""
    <span class={["tag", "tag-#{@tone}", tag_variant_class(@variant)]}>
      <i :if={@dot} aria-hidden="true"></i>
      {render_slot(@inner_block)}
    </span>
    """
  end

  defp tag_variant_class(:default), do: nil
  defp tag_variant_class(:environment), do: "environment-tag"
end
