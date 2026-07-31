defmodule HephaestusWebWeb.DesignSystem.Composites.OrganizationHeader do
  @moduledoc "Organization identity, hierarchy, and peer navigation."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem,
    only: [breadcrumbs: 1, frame: 1, tab_navigation: 1, text: 1]

  attr :organization, :map, required: true
  attr :active, :atom, required: true, values: [:projects, :secrets]
  attr :index_destination, :string, default: "/organizations"
  attr :projects_destination, :string, default: nil
  attr :secrets_destination, :string, default: nil

  @doc "Renders an organization header using only public design-system APIs."
  def organization_header(assigns) do
    id = assigns.organization["id"]
    name = assigns.organization["name"]

    assigns =
      assigns
      |> assign(:name, name)
      |> assign(:initial, name |> String.first() |> String.upcase())
      |> assign(:projects_destination, assigns.projects_destination || "/organizations/#{id}")
      |> assign(
        :secrets_destination,
        assigns.secrets_destination || "/organizations/#{id}/secrets"
      )
      |> assign(:tabs, [
        %{
          key: :projects,
          label: "Projects",
          icon: "hero-squares-2x2",
          destination: assigns.projects_destination || "/organizations/#{id}"
        },
        %{
          key: :secrets,
          label: "Secrets",
          icon: "hero-key",
          destination: assigns.secrets_destination || "/organizations/#{id}/secrets"
        }
      ])

    ~H"""
    <.breadcrumbs id="organization-breadcrumbs">
      <:item navigate={@index_destination}>Organizations</:item>
      <:current>{@name}</:current>
    </.breadcrumbs>
    <.frame as="section" variant={:organization_header}>
      <.frame variant={:organization_mark}>{@initial}</.frame>
      <.frame variant={:organization_body}>
        <.text as="p" variant={:eyebrow}>Organization workspace</.text>
        <.text as="h1" variant={:title}>{@name}</.text>
        <.text as="p" variant={:lede}>
          Projects, reusable agents, and delegated credentials in one perimeter.
        </.text>
      </.frame>
    </.frame>
    <.tab_navigation id="organization-tabs" label="Organization" items={@tabs} active={@active} />
    """
  end
end
