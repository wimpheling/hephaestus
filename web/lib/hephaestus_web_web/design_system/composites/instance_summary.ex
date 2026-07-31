defmodule HephaestusWebWeb.DesignSystem.Composites.InstanceSummary do
  @moduledoc "Agent instance identity, release, attachment, and run summary."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [action: 1, frame: 1, tag: 1, text: 1]

  attr :id, :string, required: true
  attr :name, :string, required: true
  attr :state, :string, required: true, values: ~w(active disabled invalid updating)
  attr :release, :string, required: true
  attr :attachments, :integer, default: 0
  attr :runs, :integer, default: 0
  attr :destination, :string, default: nil

  @doc "Renders an instance summary with a single explicit destination."
  def instance_summary(assigns) do
    ~H"""
    <.frame as="article" id={@id} variant={:summary}>
      <.frame as="header" variant={:summary_header}>
        <.text as="h3" variant={:title}>
          <.action :if={@destination} destination={@destination}>{@name}</.action>
          <.text :if={!@destination} as="span">{@name}</.text>
        </.text>
        <.tag tone={tone(@state)}>{@state}</.tag>
      </.frame>
      <.frame variant={:metadata}>
        <.text as="span">
          <.text as="strong">Release</.text>
          {@release}
        </.text>
        <.text as="span">
          <.text as="strong">Attachments</.text>
          {@attachments}
        </.text>
        <.text as="span">
          <.text as="strong">Runs</.text>
          {@runs}
        </.text>
      </.frame>
    </.frame>
    """
  end

  defp tone("active"), do: "success"
  defp tone("invalid"), do: "danger"
  defp tone("updating"), do: "warning"
  defp tone(_state), do: "neutral"
end
