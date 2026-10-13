defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectPage do
  @moduledoc "Pure presentation for project repositories."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project, :map, default: nil
  attr :project_id, :string, required: true
  attr :item_count, :integer, default: 0
  attr :repositories, :any, default: []

  attr :recipe_form, :any, default: nil

  attr :recipe_inspection, :map,
    default: %{status: :idle, deployment_id: "", deployment: nil, error: nil, generation: 0}

  attr :installed_ui, :map,
    default: %{
      state: :loading,
      installations: [],
      error: nil,
      has_more: false,
      loading_more: false
    }

  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true
  attr :repository_destination, :any, required: true

  @doc "Renders the project repository collection."
  def project_page(assigns) do
    ~H"""
    <.page_state
      id="project-repositories-page-state"
      state={@state}
      title="Repositories unavailable"
      message="The project repository collection is not ready."
    >
      <.frame variant={:summary_body}>
        <.project_header
          project={@project}
          project_id={@project_id}
          item_count={@item_count}
          active={:repositories}
          organization_index_destination={@organization_index_destination}
          organization_destination={@organization_destination}
        />
        <.frame as="section" id="project-repositories" variant={:table}>
          <.resource_list id="project-repository-stream" layout={:projects} update="stream">
            <:header>
              <.text as="span" variant={:muted}>Repository</.text>
              <.text as="span" variant={:muted}>Branch</.text>
              <.text as="span" variant={:muted}>Agents</.text>
              <.text as="span" variant={:muted}>Runs</.text>
            </:header>
            <:empty>No visible repositories in this project.</:empty>
            <.action
              :for={{dom_id, repository} <- @repositories}
              id={dom_id}
              destination={@repository_destination.(repository["id"])}
              variant={:resource_row}
            >
              <.frame variant={:resource_primary}>
                <.glyph name="hero-command-line" />
                <.frame variant={:resource_detail}>
                  <.text as="strong">{repository["name"]}</.text>
                  <.text as="small" variant={:muted}>
                    {if(repository["is_public"], do: "public", else: "private")}
                  </.text>
                </.frame>
              </.frame>
              <.text as="code" variant={:mono}>{repository["default_branch"]}</.text>
              <.text as="span">{repository["attachment_count"]}</.text>
              <.text as="span">{repository["run_count"]}</.text>
            </.action>
          </.resource_list>
        </.frame>
        <.recipe_deployment_inspection state={@recipe_inspection} form={@recipe_form} />
        <.installed_ui_navigation
          scope={:project}
          state={@installed_ui.state}
          installations={@installed_ui.installations}
          error={@installed_ui.error}
          has_more={@installed_ui.has_more}
          loading_more={@installed_ui.loading_more}
        />
      </.frame>
    </.page_state>
    """
  end

  attr :state, :map, required: true

  attr :form, :any, default: nil

  defp recipe_deployment_inspection(assigns) do
    ~H"""
    <.frame as="section" id="recipe-deployment-inspection" variant={:summary_body}>
      <.text as="h2">Inspect a recipe deployment</.text>
      <.text as="p" variant={:muted}>
        Enter an existing deployment ID to read its saved progress.
      </.text>
      <.form_container
        :if={@form}
        for={@form}
        id="inspect-recipe-deployment-form"
        submit="inspect-recipe-deployment"
      >
        <.input
          field={@form[:deployment_id]}
          label="Deployment ID"
          required
          maxlength="36"
          autocomplete="off"
        />
        <.action interaction={:submit} variant={:secondary}>Inspect deployment</.action>
      </.form_container>
      <.text :if={@state.status == :loading} as="p">Reading saved progress…</.text>
      <.text :if={@state.error} as="p" variant={:status}>{@state.error}</.text>
      <.frame :if={@state.status == :ready && @state.deployment} variant={:resource_detail}>
        <.text as="strong">{@state.deployment["key"]}</.text>
        <.text as="p">
          Deployment {@state.deployment["id"]} · Version {@state.deployment["version"]} · {@state.deployment[
            "lifecycle"
          ]}
        </.text>
        <.text as="p">
          Activation is unobserved. Saved Ready or Installed progress does not confirm a running workload.
        </.text>
        <.frame :for={resource <- @state.deployment["resources"]} variant={:resource_detail}>
          <.text as="strong">{resource["name"]} · {resource["kind"]} · {resource["ownership"]}</.text>
          <.text as="code">{resource["resource_id"]}</.text>
          <.text :if={resource["revision_id"]} as="code">Revision {resource["revision_id"]}</.text>
          <.text as="p">
            Install: {resource["install"]} · Removal: {resource["removal"]} · Version {resource[
              "version"
            ]}
          </.text>
          <.text :if={resource["recovery_required"]} as="p">Held for reconciliation.</.text>
          <.text :if={resource["active_attempt_id"]} as="code">
            Attempt {resource["active_attempt_id"]}
          </.text>
          <.text :if={resource["diagnostic"] != "unspecified"} as="p">
            Diagnostic: {resource["diagnostic"]}
          </.text>
        </.frame>
      </.frame>
    </.frame>
    """
  end

  attr :project, :map, required: true
  attr :project_id, :string, required: true
  attr :item_count, :integer, required: true

  attr :active, :atom,
    required: true,
    values: [:repositories, :agents, :runs, :images, :settings]

  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true

  defp project_header(assigns) do
    ~H"""
    <.breadcrumbs id="project-breadcrumbs">
      <:item navigate={@organization_index_destination}>Organizations</:item>
      <:item navigate={@organization_destination}>{@project["organization_name"]}</:item>
      <:current>{@project["name"]}</:current>
    </.breadcrumbs>
    <.page_heading
      eyebrow="Project workspace"
      title={@project["name"]}
      description="Project-owned agent instances keep independent revisions, state, and runs."
    >
      <:actions>
        <.tag>{@item_count} visible</.tag>
        <.action
          id="create-repository-link"
          destination={"/projects/#{@project_id}/repositories/new"}
          variant={:secondary}
        >
          <.glyph name="hero-plus" /> Create repository
        </.action>
        <.action
          id="new-session-chat-link"
          destination={"/projects/#{@project_id}/session-chat/new"}
          variant={:secondary}
        >
          <.glyph name="hero-chat-bubble-left-right" /> New session chat
        </.action>
      </:actions>
    </.page_heading>
    <.tab_navigation id="project-tabs" label="Project" active={@active} items={tabs(@project_id)} />
    """
  end

  defp tabs(project_id) do
    [
      %{
        key: :repositories,
        label: "Repositories",
        icon: "hero-circle-stack",
        destination: "/projects/#{project_id}"
      },
      %{
        key: :agents,
        label: "Agents",
        icon: "hero-cpu-chip",
        destination: "/projects/#{project_id}/agents"
      },
      %{
        key: :runs,
        label: "Runs",
        icon: "hero-play-circle",
        destination: "/projects/#{project_id}/runs"
      },
      %{
        key: :images,
        label: "Images",
        icon: "hero-cube",
        destination: "/projects/#{project_id}/images"
      },
      %{
        key: :gateways,
        label: "Gateways",
        icon: "hero-globe-alt",
        destination: "/projects/#{project_id}/gateways"
      },
      %{
        key: :settings,
        label: "Settings",
        icon: "hero-cog-6-tooth",
        destination: "/projects/#{project_id}/settings"
      }
    ]
  end
end
