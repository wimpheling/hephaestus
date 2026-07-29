defmodule HephaestusWebWeb.DesignSystem do
  @moduledoc """
  Shared visual primitives for the Hephaestus browser application.

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

  attr :rest, :global

  def breadcrumbs(assigns) do
    ~H"""
    <nav class="breadcrumbs" aria-label="Breadcrumb" {@rest}>
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
  attr :class, :any, default: nil
  attr :rest, :global
  slot :inner_block, required: true

  def tag(assigns) do
    ~H"""
    <span class={["tag", "tag-#{@tone}", @class]} {@rest}>
      <i :if={@dot} aria-hidden="true"></i>
      {render_slot(@inner_block)}
    </span>
    """
  end
end
