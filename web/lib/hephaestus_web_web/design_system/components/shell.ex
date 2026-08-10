defmodule HephaestusWebWeb.DesignSystem.Components.Shell do
  @moduledoc """
  Implements the root document and authenticated application shell.
  """
  use Phoenix.Component
  use Gettext, backend: HephaestusWebWeb.Gettext

  import HephaestusWebWeb.DesignSystem.Components.Core
  import HephaestusWebWeb.DesignSystem.Components.Navigation
  import Phoenix.Controller, only: [get_csrf_token: 0]

  alias HephaestusWebWeb.DesignSystem.Interactions

  # Embed all files in layouts/* within this module.
  # The default root.html.heex file contains the HTML
  # skeleton of your application, namely HTML headers
  # and other static content.
  embed_templates "root*"

  @doc """
  Renders your app layout.

  This function is typically invoked from every template,
  and it often contains your application menu, sidebar,
  or similar.

  ## Examples

      <Layouts.app flash={@flash}>
        <h1>Content</h1>
      </Layouts.app>

  """
  attr :flash, :map, required: true, doc: "the map of flash messages"

  attr :current_identity, :map,
    default: nil,
    doc: "the authenticated browser principal"

  attr :organizations_destination, :string, required: true
  attr :logout_destination, :string, required: true
  slot :inner_block, required: true

  def app(assigns) do
    ~H"""
    <header class="app-header">
      <div class="app-header-inner">
        <a href={@organizations_destination} class="brand">
          <span class="brand-mark">H</span>
          <span>HEPHAESTUS</span>
        </a>
        <div class="header-actions">
          <.tag variant={:environment} tone="success" dot>local forge</.tag>
          <a
            :if={@current_identity}
            href="/settings/git-credentials"
            class="identity-chip"
            aria-label="Manage Git credentials"
          >
            <span>{@current_identity.display_name |> String.first() |> String.upcase()}</span>
            <div><strong>{@current_identity.display_name}</strong><small>authenticated</small></div>
          </a>
          <.link
            :if={@current_identity}
            href={@logout_destination}
            method="delete"
            class="logout-link"
          >
            Sign out
          </.link>
          <.theme_toggle />
        </div>
      </div>
    </header>

    <main class="app-main">
      {render_slot(@inner_block)}
    </main>

    <.flash_group flash={@flash} />
    """
  end

  @doc """
  Shows the flash group with standard titles and content.

  ## Examples

      <.flash_group flash={@flash} />
  """
  attr :flash, :map, required: true, doc: "the map of flash messages"
  attr :id, :string, default: "flash-group", doc: "the optional id of flash container"

  def flash_group(assigns) do
    ~H"""
    <div id={@id} aria-live="polite">
      <.flash kind={:info} flash={@flash} />
      <.flash kind={:error} flash={@flash} />

      <.flash
        id="client-error"
        kind={:error}
        title={gettext("We can't find the internet")}
        disconnected={Interactions.disconnected("#client-error")}
        connected={Interactions.connected("#client-error")}
        hidden
      >
        {gettext("Attempting to reconnect")}
        <.icon name="hero-arrow-path" size={:small} treatment={:loading} />
      </.flash>

      <.flash
        id="server-error"
        kind={:error}
        title={gettext("Something went wrong!")}
        disconnected={Interactions.disconnected("#server-error")}
        connected={Interactions.connected("#server-error")}
        hidden
      >
        {gettext("Attempting to reconnect")}
        <.icon name="hero-arrow-path" size={:small} treatment={:loading} />
      </.flash>
    </div>
    """
  end

  @doc """
  Provides dark vs light theme toggle based on themes defined in app.css.

  See <head> in root.html.heex which applies the theme before page load.
  """
  def theme_toggle(assigns) do
    ~H"""
    <div class="card relative flex flex-row items-center border-2 border-base-300 bg-base-300 rounded-full">
      <div class="absolute w-1/3 h-full rounded-full border-1 border-base-200 bg-base-100 brightness-200 left-0 [[data-theme=light]_&]:left-1/3 [[data-theme=dark]_&]:left-2/3 [[data-theme-source=system]_&]:!left-0 transition-[left]" />

      <button
        class="flex p-2 cursor-pointer w-1/3"
        aria-label={gettext("Use system theme")}
        phx-click={Interactions.set_theme()}
        data-phx-theme="system"
      >
        <.icon name="hero-computer-desktop-micro" treatment={:theme} />
      </button>

      <button
        class="flex p-2 cursor-pointer w-1/3"
        aria-label={gettext("Use light theme")}
        phx-click={Interactions.set_theme()}
        data-phx-theme="light"
      >
        <.icon name="hero-sun-micro" treatment={:theme} />
      </button>

      <button
        class="flex p-2 cursor-pointer w-1/3"
        aria-label={gettext("Use dark theme")}
        phx-click={Interactions.set_theme()}
        data-phx-theme="dark"
      >
        <.icon name="hero-moon-micro" treatment={:theme} />
      </button>
    </div>
    """
  end
end
