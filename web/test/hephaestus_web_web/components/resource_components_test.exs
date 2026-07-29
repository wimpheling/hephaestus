defmodule HephaestusWebWeb.ResourceComponentsTest do
  use ExUnit.Case, async: true

  use HephaestusWebWeb, :html

  import Phoenix.LiveViewTest

  test "resource list renders caller-owned headers and rows without domain assumptions" do
    html =
      render_component(&list_fixture/1,
        rows: [
          %{id: "first", name: "First resource", count: 2},
          %{id: "second", name: "Second resource", count: 7}
        ]
      )

    document = LazyHTML.from_fragment(html)

    assert count(document, "#generic-resources.resource-list") == 1
    assert count(document, "#generic-resources .resource-list-heading span") == 2
    assert count(document, "#generic-resources .resource-list-row") == 2
    assert text(document, "#resource-first strong") == "First resource"
    assert text(document, "#resource-second span:last-child") == "7"
    assert count(document, "#generic-resources-empty") == 0
  end

  test "resource list exposes the caller's empty copy" do
    html = render_component(&list_fixture/1, rows: [])
    document = LazyHTML.from_fragment(html)

    assert count(document, "#generic-resources .resource-list-row") == 0
    assert text(document, "#generic-resources-empty") == "Nothing here yet."
  end

  def list_fixture(assigns) do
    ~H"""
    <.resource_list id="generic-resources" columns="1fr 5rem">
      <:header><span>Name</span><span>Count</span></:header>
      <:empty :if={@rows == []}>Nothing here yet.</:empty>
      <:row :for={row <- @rows}>
        <article id={"resource-#{row.id}"} class="resource-list-row">
          <strong>{row.name}</strong><span>{row.count}</span>
        </article>
      </:row>
    </.resource_list>
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
    |> String.trim()
  end
end
