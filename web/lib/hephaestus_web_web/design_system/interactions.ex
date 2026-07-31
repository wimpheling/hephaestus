defmodule HephaestusWebWeb.DesignSystem.Interactions do
  @moduledoc "Central LiveView interaction commands consumed by basic UI components."

  alias Phoenix.LiveView.JS

  @doc "Clears one flash entry and hides its rendered notice."
  def clear_flash(kind, id), do: JS.push("lv:clear-flash", value: %{key: kind}) |> hide("##{id}")

  @doc "Shows a disconnected-status notice."
  def disconnected(selector) do
    show(".phx-client-error #{selector}")
    |> JS.remove_attribute("hidden", to: ".phx-client-error #{selector}")
  end

  @doc "Hides a notice after the LiveView connection is restored."
  def connected(selector), do: hide(selector) |> JS.set_attribute({"hidden", ""})

  @doc "Dispatches the colocated theme-selection browser event."
  def set_theme, do: JS.dispatch("phx:set-theme")

  @doc "Shows a selector with the design-system transition."
  def show(js \\ %JS{}, selector) do
    JS.show(js,
      to: selector,
      time: 300,
      transition:
        {"transition-all ease-out duration-300",
         "opacity-0 translate-y-4 sm:translate-y-0 sm:scale-95",
         "opacity-100 translate-y-0 sm:scale-100"}
    )
  end

  @doc "Hides a selector with the design-system transition."
  def hide(js \\ %JS{}, selector) do
    JS.hide(js,
      to: selector,
      time: 200,
      transition:
        {"transition-all ease-in duration-200", "opacity-100 translate-y-0 sm:scale-100",
         "opacity-0 translate-y-4 sm:translate-y-0 sm:scale-95"}
    )
  end
end
