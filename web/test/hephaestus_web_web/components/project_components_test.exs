defmodule HephaestusWebWeb.ProjectComponentsTest do
  use ExUnit.Case, async: true

  use HephaestusWebWeb, :html

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
    project_id = assigns.project_id

    assigns =
      assign(assigns, :items, [
        %{key: :repositories, label: "Repositories", destination: "/projects/#{project_id}"},
        %{key: :agents, label: "Agents", destination: "/projects/#{project_id}/agents"},
        %{key: :runs, label: "Runs", destination: "/projects/#{project_id}/runs"},
        %{key: :settings, label: "Settings", destination: "/projects/#{project_id}/settings"}
      ])

    ~H"""
    <.tab_navigation id="project-tabs" label="Project" items={@items} active={@active} />
    """
  end

  defp count(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.to_tree()
    |> length()
  end
end
