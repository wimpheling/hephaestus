defmodule HephaestusWebWeb.PageHTML do
  @moduledoc """
  This module contains pages rendered by PageController.

  See the `page_html` directory for all templates available.
  """
  use HephaestusWebWeb, :html

  alias HephaestusWebWeb.DesignSystem.Pages.HomePage

  @doc "Renders the public landing page through the design-system facade."
  def home(assigns), do: HomePage.home_page(assigns)
end
