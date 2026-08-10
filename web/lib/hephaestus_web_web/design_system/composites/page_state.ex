defmodule HephaestusWebWeb.DesignSystem.Composites.PageState do
  @moduledoc "Loading, empty, error, reconnecting, and ready presentation states."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1, glyph: 1, text: 1]

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :id, :string, required: true
  attr :state, :atom, required: true, values: @states
  attr :title, :string, default: nil
  attr :message, :string, default: nil
  slot :inner_block

  @doc "Renders a finite page state with appropriate live-region semantics."
  def page_state(assigns) do
    assigns =
      assigns
      |> assign(:role, role(assigns.state))
      |> assign(:live, live(assigns.state))
      |> assign(:default_title, default_title(assigns.state))
      |> assign(:icon, icon(assigns.state))

    ~H"""
    <.frame
      :if={@state != :ready}
      as="section"
      id={@id}
      variant={if(@state == :loading, do: :loading_page_state, else: :page_state)}
      role={@role}
      aria_live={@live}
    >
      <.glyph name={@icon} size={:large} />
      <.text as="h2" variant={:title}>{@title || @default_title}</.text>
      <.text :if={@message} as="p" variant={:muted}>{@message}</.text>
    </.frame>
    <%= if @state == :ready do %>
      {render_slot(@inner_block)}
    <% end %>
    """
  end

  def states, do: @states

  defp role(:error), do: "alert"
  defp role(_state), do: "status"
  defp live(:error), do: "assertive"
  defp live(_state), do: "polite"
  defp default_title(:loading), do: "Loading"
  defp default_title(:empty), do: "Nothing here yet"
  defp default_title(:error), do: "Unable to load this page"
  defp default_title(:reconnecting), do: "Reconnecting"
  defp default_title(:ready), do: "Ready"
  defp icon(:loading), do: "hero-arrow-path"
  defp icon(:empty), do: "hero-inbox"
  defp icon(:error), do: "hero-exclamation-triangle"
  defp icon(:reconnecting), do: "hero-signal-slash"
  defp icon(:ready), do: "hero-check-circle"
end
