defmodule HephaestusWebWeb.DesignSystem.Pages.HomePageTest do
  use ExUnit.Case, async: true
  use HephaestusWebWeb, :html

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.HomePage

  @covered_states [:ready]

  test "renders the ready landing page through the pure page boundary" do
    assert @covered_states == [:ready]

    html = render_component(&HomePage.home_page/1, state: :ready, flash: %{})

    assert html =~ "Code enters."
    assert html =~ "Sign in with OIDC"
  end
end
