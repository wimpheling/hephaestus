defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectNewPage do
  @moduledoc "Pure presentation for creating a project."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :organization, :map, default: nil
  attr :form, :any, required: true
  attr :create_event, :string, required: true

  def project_new(assigns) do
    ~H"""
    <.page_state
      state={@state}
      id="project-new-page-state"
      title="Project form unavailable"
      message="The organization project form is not ready."
    >
      <.frame :if={@state == :ready && @organization} variant={:summary_body}>
        <.breadcrumbs id="project-new-breadcrumbs">
          <:item navigate="/organizations">Organizations</:item>
          <:item navigate={"/organizations/#{@organization["id"]}"}>{@organization["name"]}</:item>
          <:current>New project</:current>
        </.breadcrumbs>
        <.page_heading
          eyebrow="Project workspace"
          title="Create project"
          description="Projects group repositories, releases, and agent instances under one authorization boundary."
        />
        <.form_container for={@form} id="create-project-form" submit={@create_event}>
          <.input field={@form[:name]} label="Project name" required autocomplete="off" />
          <.input
            field={@form[:description]}
            type="textarea"
            label="Description"
            rows="4"
            maxlength="2000"
            placeholder="What belongs in this project?"
          />
          <.action interaction={:submit} variant={:primary}>Create project</.action>
        </.form_container>
      </.frame>
    </.page_state>
    """
  end
end
