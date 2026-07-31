defmodule HephaestusWebWeb.DesignSystem.AccessibilityParityTest do
  use ExUnit.Case, async: true
  use HephaestusWebWeb, :html

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem
  alias HephaestusWebWeb.DesignSystem.Showcase

  @a11y_test_ids [
    :action,
    :app,
    :breadcrumbs,
    :button,
    :flash,
    :flash_group,
    :form_container,
    :frame,
    :glyph,
    :header,
    :icon,
    :input,
    :list,
    :repository_tree,
    :root,
    :table,
    :tag,
    :text,
    :theme_toggle,
    :build_status,
    :confirmation_flow,
    :instance_summary,
    :organization_header,
    :page_heading,
    :page_state,
    :release_provenance,
    :repository_browser,
    :repository_shell,
    :resource_list,
    :run_timeline,
    :secret_summary,
    :tab_navigation
  ]

  test "every public rendering contract has a showcase and accessibility audit" do
    catalog_ids = MapSet.new(DesignSystem.catalog(), & &1.showcase_id)
    showcase_ids = MapSet.new(Showcase.examples(), & &1.id)
    test_ids = MapSet.new(@a11y_test_ids)

    assert showcase_ids == catalog_ids
    assert test_ids == catalog_ids

    Enum.each(@a11y_test_ids, fn id ->
      html = render_component(&Showcase.example/1, %{id: id})
      assert String.trim(html) != "", "#{id} rendered no example"
      assert_accessible_markup(id, html)
    end)
  end

  test "page-state showcase contract covers every declared state" do
    Enum.each(HephaestusWebWeb.DesignSystem.Composites.PageState.states(), fn state ->
      html = render_component(&page_state_fixture/1, %{state: state})

      assert String.trim(html) != ""
    end)
  end

  test "action exposes its bounded accessible-label property" do
    html = render_component(&Showcase.example/1, %{id: :action})

    assert html =~ ~s|aria-label="View organizations"|
  end

  test "flash group keeps reconnect feedback accessible and declarative" do
    html = render_component(&Showcase.example/1, %{id: :flash_group})

    assert html =~ ~s|id="client-error"|
    assert html =~ ~s|role="alert"|
    assert html =~ "Attempting to reconnect"
    assert html =~ "phx-disconnected="
    assert html =~ "phx-connected="
  end

  defp page_state_fixture(assigns) do
    ~H"""
    <DesignSystem.page_state id={"state-#{@state}"} state={@state} message={"State #{@state}"}>
      Ready content
    </DesignSystem.page_state>
    """
  end

  defp assert_accessible_markup(id, html) do
    tree = html |> LazyHTML.from_fragment() |> LazyHTML.to_tree()

    Enum.each(elements(tree, ["a", "button"]), fn {tag, attrs, children} ->
      accessible_name = attribute(attrs, "aria-label") || text_content(children)

      assert String.trim(accessible_name) != "",
             "#{id} rendered <#{tag}> without an accessible name"
    end)

    Enum.each(elements(tree, ["nav"]), fn {_tag, attrs, _children} ->
      assert present?(attribute(attrs, "aria-label")),
             "#{id} rendered navigation without aria-label"
    end)

    labels = elements(tree, ["label"])

    Enum.each(elements(tree, ["input", "select", "textarea"]), fn {tag, attrs, _children} ->
      unless attribute(attrs, "type") == "hidden" do
        control_id = attribute(attrs, "id")

        assert present?(control_id), "#{id} rendered <#{tag}> without an id"

        assert Enum.any?(labels, fn {_label, label_attrs, label_children} ->
                 attribute(label_attrs, "for") == control_id or
                   Enum.any?(elements(label_children, [tag]), fn {_control, nested_attrs, _nested} ->
                     attribute(nested_attrs, "id") == control_id
                   end)
               end),
               "#{id} rendered <#{tag}> without an associated label"
      end
    end)
  end

  defp elements(tree, names) when is_list(tree),
    do: Enum.flat_map(tree, &elements(&1, names))

  defp elements({tag, _attrs, children} = node, names) when is_binary(tag) do
    own = if tag in names, do: [node], else: []
    own ++ elements(children, names)
  end

  defp elements(_node, _names), do: []

  defp text_content(nodes) when is_list(nodes), do: Enum.map_join(nodes, &text_content/1)
  defp text_content({_tag, _attrs, children}), do: text_content(children)
  defp text_content(text) when is_binary(text), do: text
  defp text_content(_node), do: ""

  defp attribute(attrs, name) do
    case List.keyfind(attrs, name, 0) do
      {^name, value} -> value
      nil -> nil
    end
  end

  defp present?(value), do: is_binary(value) and String.trim(value) != ""
end
