defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectSettingsPage do
  @moduledoc "Pure presentation for project secret settings."
  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem
  @states [:loading, :empty, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :project, :map, default: nil
  attr :project_id, :string, required: true
  attr :item_count, :integer, default: 0
  attr :secrets, :any, default: []
  attr :project_secrets, :list, default: []
  attr :secret_authority, :map, default: %{"grants" => [], "imports" => []}
  attr :project_repositories, :list, default: []
  attr :form, :map, default: %{}
  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true
  attr :create_secret_event, :string, required: true, values: ["create-secret"]
  attr :rotate_secret_event, :string, required: true, values: ["rotate-secret"]
  attr :set_secret_enabled_event, :string, required: true, values: ["set-secret-enabled"]
  attr :revoke_secret_event, :string, required: true, values: ["revoke-secret"]
  attr :purge_secret_event, :string, required: true, values: ["purge-secret"]
  attr :grant_secret_event, :string, required: true, values: ["grant-secret"]
  attr :accept_import_event, :string, required: true, values: ["accept-secret-import"]

  @doc "Renders write-only project secret settings and declared controls."
  def project_settings_page(assigns) do
    ~H"""
    <.page_state
      id="project-settings-page-state"
      state={@state}
      title="Settings unavailable"
      message="Project settings are not ready."
    >
      <.frame variant={:summary_body}>
        <.breadcrumbs id="project-breadcrumbs">
          <:item navigate={@organization_index_destination}>Organizations</:item><:item navigate={
            @organization_destination
          }>
            {@project["organization_name"]}
          </:item><:current>{@project["name"]}</:current>
        </.breadcrumbs>
        <.page_heading
          eyebrow="Project workspace"
          title={@project["name"]}
          description="Project-owned write-only secret authority."
        >
          <:actions>
            <.tag>{@item_count} visible</.tag>
          </:actions>
        </.page_heading>
        <.tab_navigation
          id="project-tabs"
          label="Project"
          active={:settings}
          items={tabs(@project_id)}
        />
        <.form_container
          for={to_form(@form.secret || %{}, as: :secret)}
          id="create-project-secret"
          submit={@create_secret_event}
        >
          <.input name="secret[name]" value="" label="Secret name" required />
          <.input
            name="secret[value]"
            value=""
            type="password"
            label="Value"
            required
            autocomplete="new-password"
          />
          <.input
            name="secret[modes][]"
            type="select"
            value={[]}
            label="Allowed delivery modes"
            options={[{"Brokered", "brokered"}, {"Raw", "raw"}]}
            multiple
            required
          />
          <.action interaction={:submit} variant={:primary}>Encrypt and create</.action>
        </.form_container>
        <.resource_list id="project-secret-stream" layout={:secrets} update="stream">
          <:header>
            <.text as="span" variant={:muted}>Secret</.text><.text as="span" variant={:muted}>
              Status
            </.text><.text as="span" variant={:muted}>Authority</.text><.text
              as="span"
              variant={:muted}
            >
              Controls
            </.text>
          </:header>
          <:empty>No project-owned secrets.</:empty>
          <.frame
            :for={{dom_id, secret} <- @secrets}
            as="article"
            id={dom_id}
            variant={:resource_row_tall}
          >
            <.text as="strong">{secret["name"]}</.text><.tag>{secret["status"]}</.tag><.text as="span">
              {secret["grant_count"]} grants
            </.text>
            <.frame variant={:resource_controls}>
              <.action
                id={"toggle-secret-#{secret["id"]}"}
                interaction={:event}
                event={@set_secret_enabled_event}
                event_payload={
                  %{secret_id: secret["id"], enabled: to_string(secret["status"] == "disabled")}
                }
                variant={:compact}
              >
                Toggle
              </.action>
              <.form_container
                for={to_form(@form.rotate || %{}, as: :rotate)}
                id={"rotate-project-secret-#{secret["id"]}"}
                submit={@rotate_secret_event}
                layout={:inline}
              >
                <.input name="rotate[secret_id]" type="hidden" value={secret["id"]} /><.input
                  name="rotate[active_version_id]"
                  type="hidden"
                  value={secret["active_version_id"]}
                /><.input
                  name="rotate[value]"
                  value=""
                  type="password"
                  aria_label="Replacement value"
                /><.action interaction={:submit} variant={:compact}>Rotate</.action>
              </.form_container>
              <.action
                interaction={:event}
                event={@revoke_secret_event}
                event_payload={%{secret_id: secret["id"]}}
                variant={:danger_compact}
              >
                Revoke
              </.action>
              <.action
                :if={secret["can_purge"]}
                interaction={:event}
                event={@purge_secret_event}
                event_payload={%{secret_id: secret["id"]}}
                variant={:danger_compact}
              >
                Purge
              </.action>
            </.frame>
          </.frame>
        </.resource_list>
        <.form_container
          for={to_form(@form.grant || %{}, as: :grant)}
          id="grant-secret"
          submit={@grant_secret_event}
        >
          <.input
            name="grant[secret_id]"
            value=""
            type="select"
            label="Secret"
            prompt="Choose a secret"
            options={Enum.map(@project_secrets, &{&1["name"], &1["id"]})}
            required
          /><.input
            name="grant[target]"
            value=""
            type="select"
            label="Exact target"
            prompt="Choose an exact target"
            options={repository_target_options(@project_repositories)}
            required
          />
          <.input
            name="grant[modes][]"
            type="select"
            value={[]}
            label="Delivery modes"
            options={[{"Brokered", "brokered"}, {"Raw", "raw"}]}
            multiple
          />
          <.input
            name="grant[phases][]"
            type="select"
            value={[]}
            label="Phases"
            options={[{"Normal", "normal"}, {"Preflight", "preflight"}, {"Update", "update"}]}
            multiple
          />
          <.input name="grant[destinations]" value="" label="Destinations" />
          <.input
            name="grant[expires_at]"
            value=""
            type="datetime-local"
            label="Expires at"
          />
          <.action interaction={:submit} variant={:primary}>Review and offer grant</.action>
        </.form_container>
        <.frame
          :for={grant <- pending_grants(@secret_authority)}
          as="article"
          id={"pending-grant-#{grant["id"]}"}
          variant={:proposal}
        >
          <.text as="strong">{grant["secret_name"]}</.text>
          <.form_container
            for={to_form(@form.import || %{}, as: :secret_import)}
            id={"accept-import-#{grant["id"]}"}
            submit={@accept_import_event}
          >
            <.input
              name="secret_import[grant_id]"
              type="hidden"
              value={grant["id"]}
            />
            <.input name="secret_import[alias]" value="" label="Alias" required />
            <.action interaction={:submit} variant={:primary}>Accept live reference</.action>
          </.form_container>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp tabs(id),
    do: [
      %{
        key: :repositories,
        label: "Repositories",
        icon: "hero-circle-stack",
        destination: "/projects/#{id}"
      },
      %{
        key: :agents,
        label: "Agents",
        icon: "hero-cpu-chip",
        destination: "/projects/#{id}/agents"
      },
      %{
        key: :builders,
        label: "Builders",
        icon: "hero-cube",
        destination: "/projects/#{id}/builders"
      },
      %{key: :runs, label: "Runs", icon: "hero-play-circle", destination: "/projects/#{id}/runs"},
      %{
        key: :settings,
        label: "Settings",
        icon: "hero-cog-6-tooth",
        destination: "/projects/#{id}/settings"
      }
    ]

  defp pending_grants(authority) do
    authority
    |> Map.get("grants", [])
    |> Enum.reject(& &1["import_id"])
  end

  defp repository_target_options(repositories) do
    Enum.map(repositories, &{"Repository · #{&1["name"]}", "repository:#{&1["id"]}"})
  end
end
