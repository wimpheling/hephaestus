defmodule HephaestusWebWeb.DesignSystemTest do
  use ExUnit.Case, async: true

  use HephaestusWebWeb, :html

  import Phoenix.LiveViewTest

  test "breadcrumbs link every ancestor and leave the current page unlinked" do
    html = render_component(&breadcrumb_fixture/1)
    document = LazyHTML.from_fragment(html)

    assert count(document, "#test-breadcrumbs a") == 2
    assert count(document, ~s|a[href="/organizations"]|) == 1

    assert count(document, ~s|a[href="/organizations/org-id"]|) == 1

    assert count(document, ~s|[aria-current="page"]|) == 1

    assert count(document, ~s|[aria-current="page"] a|) == 0
  end

  test "tag exposes a reusable tone and optional status dot" do
    html = render_component(&tag_fixture/1)
    document = LazyHTML.from_fragment(html)

    assert count(document, ".tag.tag-success") == 1
    assert count(document, ".tag.tag-success i") == 1
  end

  def breadcrumb_fixture(assigns) do
    ~H"""
    <.breadcrumbs id="test-breadcrumbs">
      <:item navigate="/organizations">Organizations</:item>
      <:item navigate="/organizations/org-id">Acme</:item>
      <:current>Forge</:current>
    </.breadcrumbs>
    """
  end

  def tag_fixture(assigns) do
    ~H"""
    <.tag tone="success" dot>Running</.tag>
    """
  end

  defp count(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.to_tree()
    |> length()
  end
end
