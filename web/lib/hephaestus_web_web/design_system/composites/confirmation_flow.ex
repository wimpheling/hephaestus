defmodule HephaestusWebWeb.DesignSystem.Composites.ConfirmationFlow do
  @moduledoc "Explicit confirmation copy and bounded destructive action."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [action: 1, frame: 1, text: 1]

  attr :id, :string, required: true
  attr :title, :string, required: true
  attr :message, :string, required: true
  attr :confirm, :string, required: true
  attr :event, :string, required: true, values: ["remove-attachment"]
  attr :label, :string, required: true
  attr :disabled, :boolean, default: false
  slot :cancel

  @doc "Renders a confirmation region whose destructive action always carries confirmation text."
  def confirmation_flow(assigns) do
    ~H"""
    <.frame as="section" id={@id} variant={:confirmation} role="alert" aria_label={@title}>
      <.text as="h3" variant={:title}>{@title}</.text>
      <.text as="p">{@message}</.text>
      <.frame variant={:page_heading_actions}>
        {render_slot(@cancel)}
        <.action
          interaction={:event}
          variant={:danger}
          event={@event}
          confirm={@confirm}
          disabled={@disabled}
        >
          {@label}
        </.action>
      </.frame>
    </.frame>
    """
  end
end
