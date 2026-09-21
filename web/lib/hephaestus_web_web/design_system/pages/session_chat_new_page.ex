defmodule HephaestusWebWeb.DesignSystem.Pages.SessionChatNewPage do
  @moduledoc "Pure presentation for creating one release-owned session chat."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project, :map, default: nil
  attr :project_id, :string, required: true
  attr :release_catalog, :list, default: []
  attr :secret_imports, :list, default: []
  attr :error, :string, default: nil
  attr :progress, :map, default: %{}
  attr :attempt_id, :string, default: nil
  attr :form, :any, required: true
  attr :create_event, :string, required: true

  def session_chat_new(assigns) do
    ~H"""
    <.page_state
      state={@state}
      id="session-chat-new-page-state"
      title="Session chat unavailable"
      message="The project session chat form is not ready."
    >
      <.frame :if={@state == :ready && @project} variant={:summary_body}>
        <.breadcrumbs id="session-chat-new-breadcrumbs">
          <:item navigate="/organizations">Organizations</:item>
          <:item navigate={"/organizations/#{@project["organization_id"]}"}>
            {@project["organization_name"]}
          </:item>
          <:item navigate={"/projects/#{@project_id}"}>{@project["name"]}</:item>
          <:current>New session chat</:current>
        </.breadcrumbs>
        <.page_heading
          eyebrow="Release-owned session"
          title="New session chat"
          description="Create a repository, bind the selected release, and open its chat UI."
        />
        <.text :if={@error} as="p" variant={:muted}>{@error}</.text>
        <.text :if={@progress != %{}} as="small" variant={:muted}>
          Existing setup resources will be reused when you retry this attempt.
        </.text>
        <.form_container for={@form} id="create-session-chat-form" submit={@create_event}>
          <.input field={@form[:setup_attempt_id]} type="hidden" value={@attempt_id} />
          <.input field={@form[:repository_name]} label="Repository name" required autocomplete="off" />
          <.input field={@form[:default_branch]} label="Default branch" required value="main" />
          <.input field={@form[:instance_name]} label="Session name" required autocomplete="off" />
          <.input
            field={@form[:release_agent_id]}
            type="select"
            label="Reference release"
            options={release_options(@release_catalog)}
            required
          />
          <.input
            field={@form[:model_rule_id]}
            label="Model rule UUID"
            required
            autocomplete="off"
          />
          <.input
            field={@form[:model_import_id]}
            type="select"
            label="Authorized model import"
            options={import_options(@secret_imports)}
            required
          />
          <.input
            field={@form[:acknowledge_repository_git_access]}
            type="checkbox"
            label="Allow the release-owned UI to access this repository through the installed origin"
            required
          />
          <.action interaction={:submit} variant={:primary}>Create and open chat</.action>
        </.form_container>
        <.installed_ui_navigation
          scope={:repository}
          state={:ready}
          installations={[]}
          has_more={false}
          loading_more={false}
        />
      </.frame>
    </.page_state>
    """
  end

  defp release_options(catalog), do: Enum.map(catalog, &{&1["display_name"], &1["id"]})

  defp import_options(imports),
    do: Enum.map(imports, &{&1["alias"] || &1["secret_name"], &1["id"]})
end
