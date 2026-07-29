defmodule HephaestusWebWeb.OrganizationComponentsTest do
  use ExUnit.Case, async: true

  use HephaestusWebWeb, :html

  import HephaestusWebWeb.OrganizationComponents
  import Phoenix.LiveViewTest

  @organization %{
    "id" => "018f689a-a81d-7c2e-943f-3a41f7981234",
    "name" => "Acme Research"
  }

  test "organization header keeps routed tabs and identity together" do
    html =
      render_component(&header_fixture/1,
        organization: @organization,
        active: :secrets
      )

    document = LazyHTML.from_fragment(html)

    assert text(document, ".organization-hero h1") == "Acme Research"
    assert count(document, "#organization-tabs a") == 2
    assert count(document, ~s|#organization-tabs a[aria-current="page"]|) == 1

    assert count(
             document,
             ~s|a[href="/organizations/#{@organization["id"]}"]|
           ) == 1

    assert count(
             document,
             ~s|a[href="/organizations/#{@organization["id"]}/secrets"]|
           ) == 1
  end

  def header_fixture(assigns) do
    ~H"""
    <.organization_header organization={@organization} active={@active} />
    """
  end

  defp count(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.to_tree()
    |> length()
  end

  defp text(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.text()
  end
end
