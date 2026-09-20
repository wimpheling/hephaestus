defmodule HephaestusWebWeb.DesignSystem.Composites.InstalledUiNavigation do
  @moduledoc "Safe installed UI navigation and one-shot embedding surface."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem, only: [frame: 1, glyph: 1, page_state: 1, tag: 1, text: 1]

  @states [:loading, :ready, :error, :access_revoked, :terminated]
  @scopes [:organization, :project, :repository]

  attr :scope, :atom, required: true, values: @scopes
  attr :state, :atom, required: true, values: @states
  attr :installations, :list, default: []
  attr :error, :string, default: nil
  attr :event, :string, default: "launch-installed-ui"
  attr :has_more, :boolean, default: false
  attr :loading_more, :boolean, default: false
  attr :load_event, :string, default: "load-more-installed-ui"

  @doc "Renders safe installed UI cards and an isolated one-shot launch host."
  def installed_ui_navigation(assigns) do
    assigns =
      assigns
      |> assign(:namespace, ui_namespace())
      |> assign(:platform_origin, platform_origin())
      |> assign(:frame_port, ui_frame_port())
      |> assign(:frame_src, frame_src())

    ~H"""
    <section
      id={"installed-ui-navigation-#{@scope}"}
      class="mt-8 space-y-4"
      aria-label="Installed UIs"
      data-ui-frame-src={@frame_src}
      data-ui-port={@frame_port}
    >
      <.frame variant={:section_heading}>
        <.frame variant={:section_heading}>
          <.text as="p" variant={:eyebrow}>Installed tools</.text>
          <.text as="h2" variant={:title}>Workspace UIs</.text>
          <.text as="p" variant={:muted}>
            Launch an authorized release UI in a separate browser surface.
          </.text>
        </.frame>
        <.tag :if={@state == :ready}>{length(@installations)} available</.tag>
      </.frame>

      <.page_state
        :if={@state != :ready}
        id={"installed-ui-state-#{@scope}"}
        state={page_state_kind(@state)}
        title={state_title(@state)}
        message={@error || state_message(@state)}
      />

      <div
        :if={@state == :ready}
        id={"installed-ui-list-#{@scope}"}
        class="grid gap-3 md:grid-cols-2"
      >
        <article
          :for={installation <- @installations}
          id={"installed-ui-#{installation["installation_id"]}"}
          class="card border border-base-300 bg-base-100 p-4 shadow-sm"
          data-ui-installation
        >
          <div class="flex items-start justify-between gap-3">
            <div class="flex min-w-0 items-start gap-3">
              <.glyph name={icon_name(installation["icon"])} />
              <div class="min-w-0">
                <.text as="strong">{installation["label"] || installation["ui_key"]}</.text>
                <.text as="small" variant={:muted}>{installation["ui_key"]}</.text>
              </div>
            </div>
            <.tag tone={lifecycle_tone(installation["lifecycle"])}>
              {lifecycle_label(installation["lifecycle"])}
            </.tag>
          </div>
          <div class="mt-4 flex items-center justify-between gap-3">
            <.text as="small" variant={:muted}>
              {presentation_label(installation["presentation"])}
            </.text>
            <button
              :if={launchable?(installation)}
              type="button"
              class="btn btn-primary btn-sm"
              phx-click={@event}
              phx-value-id={installation["installation_id"]}
              aria-label={"Launch #{installation["label"] || installation["ui_key"]}"}
            >
              Launch
            </button>
            <.text :if={!launchable?(installation)} as="small" variant={:muted}>
              Unavailable
            </.text>
          </div>
        </article>
        <.frame :if={@installations == []} id={"installed-ui-empty-#{@scope}"} variant={:panel}>
          <.text as="p" variant={:muted}>No installed UIs are available in this scope.</.text>
        </.frame>
      </div>

      <button
        :if={@state == :ready && @has_more}
        type="button"
        class="btn btn-outline btn-sm"
        phx-click={@load_event}
        disabled={@loading_more}
      >
        {if @loading_more, do: "Loading…", else: "Load more"}
      </button>

      <section
        id={"installed-ui-embed-#{@scope}"}
        class="hidden overflow-hidden rounded-xl border border-base-300 bg-base-100 shadow-lg"
        phx-hook="InstalledUiNavigation"
        phx-update="ignore"
        data-ui-namespace={@namespace}
        data-ui-platform-origin={@platform_origin}
        data-ui-frame-src={@frame_src}
        data-ui-port={@frame_port}
      >
        <div class="flex items-center justify-between border-b border-base-300 px-4 py-3">
          <p data-ui-status role="status" aria-live="polite">Loading installed UI…</p>
          <button type="button" class="btn btn-ghost btn-sm" data-ui-close>Close</button>
        </div>
        <iframe
          id={"installed-ui-frame-#{@scope}"}
          class="h-[min(70vh,48rem)] w-full"
          title="Installed UI"
          sandbox="allow-scripts allow-same-origin"
          referrerpolicy="no-referrer"
          data-ui-frame
        />
      </section>

      <p
        id={"installed-ui-terminal-status-#{@scope}"}
        class="hidden rounded-lg border border-warning/40 bg-warning/10 px-4 py-3 text-sm"
        data-ui-terminal-status
        role="status"
        aria-live="polite"
      />
    </section>
    """
  end

  defp page_state_kind(:access_revoked), do: :error
  defp page_state_kind(:terminated), do: :error
  defp page_state_kind(state), do: state

  defp state_title(:loading), do: "Loading installed UIs"
  defp state_title(:error), do: "Installed UIs unavailable"
  defp state_title(:access_revoked), do: "Installed UI access revoked"
  defp state_title(:terminated), do: "Installed UI access ended"
  defp state_title(_state), do: "Installed UIs unavailable"

  defp state_message(:loading), do: "Checking current organization access."
  defp state_message(:error), do: "The current installed UI projection could not be loaded."
  defp state_message(:access_revoked), do: "Refresh this workspace to request current access."
  defp state_message(:terminated), do: "The embedded UI has been closed."
  defp state_message(_state), do: "No installed UIs are available in this scope."

  defp launchable?(%{"lifecycle" => "enabled", "launchable" => true}), do: true
  defp launchable?(_installation), do: false

  defp lifecycle_label("enabled"), do: "Enabled"
  defp lifecycle_label("disabled"), do: "Disabled"
  defp lifecycle_label("removed"), do: "Removed"
  defp lifecycle_label(_lifecycle), do: "Unavailable"

  defp lifecycle_tone("enabled"), do: "success"
  defp lifecycle_tone("disabled"), do: "neutral"
  defp lifecycle_tone(_lifecycle), do: "warning"

  defp presentation_label("full_page"), do: "Full page"
  defp presentation_label("iframe"), do: "Embedded"
  defp presentation_label(_presentation), do: "Unavailable"

  defp icon_name("chat"), do: "hero-chat-bubble-left-right"
  defp icon_name("code"), do: "hero-code-bracket"
  defp icon_name("book"), do: "hero-book-open"
  defp icon_name("chart"), do: "hero-chart-bar"
  defp icon_name(_icon), do: "hero-squares-2x2"

  defp ui_namespace do
    :hephaestus_web
    |> Application.get_env(:ui_browser, [])
    |> Keyword.get(:namespace, "")
  end

  defp platform_origin do
    config = Application.get_env(:hephaestus_web, :ui_browser, [])
    configured = Keyword.get(config, :platform_origin)
    host = Keyword.get(config, :platform_host, "")

    configured || "https://#{host}"
  end

  defp ui_frame_port do
    :hephaestus_web
    |> Application.get_env(:ui_browser, [])
    |> Keyword.get(:port, 443)
    |> to_string()
  end

  defp frame_src do
    config = Application.get_env(:hephaestus_web, :ui_browser, [])
    namespace = Keyword.get(config, :namespace)
    port = Keyword.get(config, :port)

    if is_binary(namespace) and namespace != "" do
      authority = if port in [nil, "", 443, "443"], do: namespace, else: "#{namespace}:#{port}"
      "https://*.#{authority}"
    else
      ""
    end
  end
end
