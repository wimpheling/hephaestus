defmodule HephaestusWebWeb.ProjectComponentsTest do
  use ExUnit.Case, async: true

  use HephaestusWebWeb, :html

  import HephaestusWebWeb.ProjectComponents
  import Phoenix.LiveViewTest

  @project_id "018f689a-a81d-7c2e-943f-3a41f7981234"

  test "project tabs are deep links with one current page" do
    html =
      render_component(&tabs_fixture/1,
        project_id: @project_id,
        active: :agents
      )

    document = LazyHTML.from_fragment(html)

    assert count(document, "#project-tabs a") == 4
    assert count(document, ~s|#project-tabs a[aria-current="page"]|) == 1
    assert count(document, ~s|a[href="/projects/#{@project_id}"]|) == 1
    assert count(document, ~s|a[href="/projects/#{@project_id}/agents"]|) == 1
    assert count(document, ~s|a[href="/projects/#{@project_id}/runs"]|) == 1
    assert count(document, ~s|a[href="/projects/#{@project_id}/settings"]|) == 1
  end

  def tabs_fixture(assigns) do
    ~H"""
    <.project_tabs project_id={@project_id} active={@active} />
    """
  end

  defp count(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.to_tree()
    |> length()
  end
end
