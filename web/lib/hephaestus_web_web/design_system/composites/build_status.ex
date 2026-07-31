defmodule HephaestusWebWeb.DesignSystem.Composites.BuildStatus do
  @moduledoc "Build identity, lifecycle status, and optional supporting facts."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1, tag: 1, text: 1]

  attr :id, :string, required: true
  attr :build_id, :string, required: true
  attr :state, :string, required: true, values: ~w(queued running succeeded failed cancelled)
  attr :commit, :string, default: nil
  slot :details

  @doc "Renders a build status with a text equivalent for its visual state."
  def build_status(assigns) do
    ~H"""
    <.frame as="article" id={@id} variant={:summary} aria_label={"Build #{@build_id}"}>
      <.frame as="header" variant={:summary_header}>
        <.text as="h3" variant={:title}>Build {@build_id}</.text>
        <.tag tone={tone(@state)} dot>{@state}</.tag>
      </.frame>
      <.text :if={@commit} as="code" variant={:mono}>{@commit}</.text>
      <.frame :if={@details != []} variant={:summary_body}>{render_slot(@details)}</.frame>
    </.frame>
    """
  end

  defp tone("succeeded"), do: "success"
  defp tone("failed"), do: "danger"
  defp tone("running"), do: "warning"
  defp tone(_state), do: "neutral"
end
