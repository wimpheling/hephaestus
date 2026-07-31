defmodule HephaestusWebWeb.DesignSystem.Composites.ReleaseProvenance do
  @moduledoc "Immutable release identity and source/build provenance."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [action: 1, frame: 1, tag: 1, text: 1]

  attr :id, :string, required: true
  attr :version, :string, required: true
  attr :state, :string, required: true, values: ~w(draft building published failed revoked)
  attr :source_commit, :string, required: true
  attr :build_id, :string, required: true
  attr :manifest_hash, :string, default: nil
  attr :source_destination, :string, default: nil
  attr :build_destination, :string, default: nil

  @doc "Renders immutable release provenance with explicit link destinations."
  def release_provenance(assigns) do
    ~H"""
    <.frame as="section" id={@id} variant={:summary} aria_label={"Release #{@version} provenance"}>
      <.frame as="header" variant={:summary_header}>
        <.text as="h2" variant={:title}>Release {@version}</.text>
        <.tag tone={tone(@state)}>{@state}</.tag>
      </.frame>
      <.frame variant={:metadata}>
        <.text as="span">
          <.text as="strong">Source</.text>
          <.action :if={@source_destination} destination={@source_destination}>
            {@source_commit}
          </.action>
          <.text :if={!@source_destination} as="code" variant={:mono}>{@source_commit}</.text>
        </.text>
        <.text as="span">
          <.text as="strong">Build</.text>
          <.action :if={@build_destination} destination={@build_destination}>{@build_id}</.action>
          <.text :if={!@build_destination} as="code" variant={:mono}>{@build_id}</.text>
        </.text>
        <.text :if={@manifest_hash} as="span">
          <.text as="strong">Manifest</.text>

          <.text as="code" variant={:mono}>{@manifest_hash}</.text>
        </.text>
      </.frame>
    </.frame>
    """
  end

  defp tone("published"), do: "success"
  defp tone("failed"), do: "danger"
  defp tone("building"), do: "warning"
  defp tone(_state), do: "neutral"
end
