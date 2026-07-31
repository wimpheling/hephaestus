defmodule Fixture.DesignSystem.Pages.DashboardPage do
  use Phoenix.Component

  alias Fixture.DesignSystem

  @states [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  attr :label, :string, required: true
  attr :on_refresh, :string, values: ["refresh"], default: "refresh"

  def page(assigns) do
    ~H"""
    <%!-- A raw <section> in a HEEx comment is not markup. --%>
    <DesignSystem.card label={@label} />
    """
  end
end
