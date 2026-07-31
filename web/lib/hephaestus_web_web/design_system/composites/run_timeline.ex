defmodule HephaestusWebWeb.DesignSystem.Composites.RunTimeline do
  @moduledoc "Ordered, durable run events with cursor-friendly stable identifiers."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1, text: 1]

  attr :id, :string, required: true
  attr :events, :list, required: true

  @doc "Renders an ordered timeline whose event maps contain id, label, time, and optional detail."
  def run_timeline(assigns) do
    ~H"""
    <.frame as="ol" id={@id} variant={:timeline} aria_label="Run timeline">
      <.frame :for={event <- @events} as="li" id={event.id} variant={:timeline_item}>
        <.frame as="span" variant={:timeline_dot} />
        <.frame variant={:summary_body}>
          <.text as="strong">{event.label}</.text>
          <.text as="time" variant={:muted} datetime={event[:datetime]}>{event.time}</.text>
          <.text :if={event[:detail]} as="code" variant={:mono}>{event.detail}</.text>
        </.frame>
      </.frame>
    </.frame>
    """
  end
end
