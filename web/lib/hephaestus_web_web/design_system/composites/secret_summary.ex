defmodule HephaestusWebWeb.DesignSystem.Composites.SecretSummary do
  @moduledoc "Non-sensitive secret metadata and bounded controls."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1, tag: 1, text: 1]

  attr :id, :string, required: true
  attr :name, :string, required: true
  attr :status, :string, required: true, values: ~w(active disabled revoked tombstoned purged)
  attr :version, :integer, required: true
  attr :modes, :list, default: []
  attr :authority, :string, default: nil
  attr :bindings, :integer, default: 0
  slot :controls

  @doc "Renders metadata only; plaintext secret values have no public property."
  def secret_summary(assigns) do
    ~H"""
    <.frame as="article" id={@id} variant={:summary}>
      <.frame as="header" variant={:summary_header}>
        <.frame variant={:summary_body}>
          <.text as="h3" variant={:title}>{@name}</.text>
          <.text as="small" variant={:muted}>value unavailable by design · version {@version}</.text>
        </.frame>
        <.tag tone={tone(@status)}>{@status}</.tag>
      </.frame>
      <.frame variant={:metadata}>
        <.text as="span">
          <.text as="strong">Modes</.text>
          {Enum.join(@modes, ", ")}
        </.text>
        <.text :if={@authority} as="span">
          <.text as="strong">Authority</.text>
          {@authority}
        </.text>
        <.text as="span">
          <.text as="strong">Bindings</.text>
          {@bindings}
        </.text>
      </.frame>
      <.frame :if={@controls != []} variant={:page_heading_actions}>{render_slot(@controls)}</.frame>
    </.frame>
    """
  end

  defp tone("active"), do: "success"
  defp tone(status) when status in ["revoked", "purged"], do: "danger"
  defp tone("disabled"), do: "warning"
  defp tone(_status), do: "neutral"
end
