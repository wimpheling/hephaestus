defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagePage do
  @moduledoc "Pure redacted preparation-history presentation for one project image."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project_id, :string, required: true
  attr :image, :map, default: nil
  attr :history, :list, default: []

  def project_repository_image_page(assigns) do
    ~H"""
    <.page_state
      id="project-image-detail-state"
      state={@state}
      title="Image unavailable"
      message="Image details are not ready."
    >
      <.frame variant={:summary_body}>
        <.page_heading
          eyebrow="Project artifacts"
          title={(@image || %{})["display_name"] || "Image"}
          description="Immutable repository source and redacted preparation evidence."
        />
        <.action destination={"/projects/#{@project_id}/images"} variant={:secondary}>
          Back to images
        </.action>
        <.frame as="section" id="project-image-summary" variant={:table}>
          <.text as="p" variant={:mono}>Source: {(@image || %{})["source_revision"] || "—"}</.text>
          <.text as="p" variant={:mono}>Base: {(@image || %{})["base_image_reference"] || "—"}</.text>
          <.text as="p">Status: {(@image || %{})["status"] || "—"}</.text>
          <.text :if={(@image || %{})["image_reference"] not in [nil, ""]} as="p" variant={:mono}>
            Output: {(@image || %{})["image_reference"]}
          </.text>
          <.text :if={(@image || %{})["failure_reason"] not in [nil, ""]} as="p" variant={:muted}>
            {(@image || %{})["failure_reason"]}
          </.text>
        </.frame>
        <.frame as="section" id="project-image-history" variant={:table}>
          <.text as="h2" variant={:title}>Preparation history</.text>
          <.resource_list id="project-image-history-events" layout={:projects}>
            <:header>
              <.text as="span" variant={:sr_only}>Repository image preparation events</.text>
            </:header>
            <:empty>No preparation events are available yet.</:empty>
            <.frame :for={event <- @history} as="article" variant={:resource_row}>
              <.text as="strong">{event["phase"]}: {event["outcome"]}</.text>
              <.text :if={event["output_digest"] not in [nil, ""]} as="code" variant={:mono}>
                {event["output_digest"]}
              </.text>
              <.text :if={event["safe_reason"] not in [nil, ""]} as="span" variant={:muted}>
                {event["safe_reason"]}
              </.text>
            </.frame>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end
end
