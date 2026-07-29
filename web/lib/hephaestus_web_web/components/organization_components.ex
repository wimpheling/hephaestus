defmodule HephaestusWebWeb.OrganizationComponents do
  @moduledoc """
  Organization-scoped identity and navigation shared by every workspace page.
  """

  use HephaestusWebWeb, :html

  attr :organization, :map, required: true
  attr :active, :atom, required: true, values: [:projects, :secrets]

  def organization_header(assigns) do
    ~H"""
    <.breadcrumbs id="organization-breadcrumbs">
      <:item navigate={~p"/organizations"}>Organizations</:item>
      <:current>{@organization["name"]}</:current>
    </.breadcrumbs>

    <section class="organization-hero">
      <div class="org-mark">
        {@organization["name"] |> String.first() |> String.upcase()}
      </div>
      <div>
        <p class="eyebrow">Organization workspace</p>
        <h1>{@organization["name"]}</h1>
        <p class="lede">Projects, reusable agents, and delegated credentials in one perimeter.</p>
      </div>
    </section>

    <nav id="organization-tabs" class="repository-tabs" aria-label="Organization">
      <.link
        navigate={~p"/organizations/#{@organization["id"]}"}
        class={["repository-tab", @active == :projects && "active"]}
        aria-current={if(@active == :projects, do: "page")}
      >
        <.icon name="hero-squares-2x2" class="size-4" /> Projects
      </.link>
      <.link
        navigate={~p"/organizations/#{@organization["id"]}/secrets"}
        class={["repository-tab", @active == :secrets && "active"]}
        aria-current={if(@active == :secrets, do: "page")}
      >
        <.icon name="hero-key" class="size-4" /> Secrets
      </.link>
    </nav>
    """
  end
end
