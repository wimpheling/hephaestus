defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryNewPage do
  @moduledoc "Pure presentation for creating a repository."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project, :map, default: nil
  attr :form, :any, required: true
  attr :create_event, :string, required: true

  def repository_new(assigns) do
    ~H"""
    <.page_state
      state={@state}
      id="repository-new-page-state"
      title="Repository form unavailable"
      message="The project repository form is not ready."
    >
      <.frame :if={@state == :ready && @project} variant={:summary_body}>
        <.breadcrumbs id="repository-new-breadcrumbs">
          <:item navigate="/organizations">Organizations</:item>
          <:item navigate={"/organizations/#{@project["organization_id"]}"}>
            {@project["organization_name"]}
          </:item>
          <:item navigate={"/projects/#{@project["id"]}"}>{@project["name"]}</:item>
          <:current>New repository</:current>
        </.breadcrumbs>
        <.page_heading
          eyebrow="Git forge"
          title="Create repository"
          description="Create an empty canonical repository for an agent project."
        />
        <.form_container for={@form} id="create-repository-form" submit={@create_event}>
          <.input field={@form[:name]} label="Repository name" required autocomplete="off" />
          <.input field={@form[:default_branch]} label="Default branch" required value="main" />
          <.input field={@form[:is_public]} type="checkbox" label="Public repository" />
          <.input
            field={@form[:agent_runs_enabled]}
            type="checkbox"
            label="Enable agent builds on push"
            checked
          />
          <.action interaction={:submit} variant={:primary}>Create repository</.action>
        </.form_container>
      </.frame>
    </.page_state>
    """
  end
end
