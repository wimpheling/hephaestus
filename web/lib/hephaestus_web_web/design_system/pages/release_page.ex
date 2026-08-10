defmodule HephaestusWebWeb.DesignSystem.Pages.ReleasePage do
  @moduledoc "Pure presentation for an immutable release."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]

  attr :state, :atom, default: :loading, values: @states
  attr :release, :map, default: nil
  attr :artifacts, :any, required: true
  attr :agents, :any, required: true
  attr :organization_index_destination, :string, default: nil
  attr :organization_destination, :string, default: nil
  attr :project_destination, :string, default: nil
  attr :repository_releases_destination, :string, default: nil
  attr :source_destination, :string, default: nil
  attr :draft_version_form, :any, default: nil
  attr :set_draft_version_event, :string, default: nil, values: [nil, "set-draft-version"]
  attr :publish_event, :string, default: nil, values: [nil, "publish-release"]

  @doc "Renders release provenance and immutable contents."
  def release(assigns) do
    ~H"""
    <.page_state
      :if={@state != :ready}
      id="release-page-state"
      state={@state}
      title="Release unavailable"
      message="The release is not ready."
    />
    <.frame :if={@state == :ready} variant={:summary_body}>
      <.breadcrumbs id="release-breadcrumbs">
        <:item navigate={@organization_index_destination}>Organizations</:item>
        <:item navigate={@organization_destination}>{@release["organization_name"]}</:item>
        <:item navigate={@project_destination}>{@release["project_name"]}</:item>
        <:item navigate={@repository_releases_destination}>{@release["repository_name"]}</:item>
        <:current>{@release["version"]}</:current>
      </.breadcrumbs>

      <.page_heading
        eyebrow="Immutable reusable release"
        title={@release["version"]}
        description="Exact source, build, configuration, and artifact-manifest provenance."
      >
        <:actions>
          <.tag tone={state_tone(@release["state"])}>{@release["state"]}</.tag>
        </:actions>
      </.page_heading>

      <.frame
        :if={@release["state"] == "draft"}
        as="section"
        id="release-draft-review"
        variant={:panel}
      >
        <.page_heading
          eyebrow="Review draft"
          title="Choose the release version"
          description="Publication freezes the source, build identity, configuration, artifact manifest, and exported agent contract."
          level="h2"
        />
        <.form_container
          :if={@draft_version_form && @set_draft_version_event}
          for={@draft_version_form}
          id="draft-version-form"
          submit={@set_draft_version_event}
        >
          <.input
            name="release[version]"
            value={@draft_version_form[:version].value}
            label="Release version"
            required
          />
          <.action interaction={:submit} variant={:secondary}>Save draft version</.action>
        </.form_container>
        <.action
          :if={@publish_event}
          interaction={:event}
          event={@publish_event}
          variant={:primary}
          confirm="Publish this immutable release?"
        >
          Publish release
        </.action>
      </.frame>

      <.frame as="section" id="release-provenance" variant={:table}>
        <.frame as="article" variant={:table_row}>
          <.frame variant={:resource_primary}>
            <.glyph name="hero-code-bracket" />
            <.frame variant={:resource_detail}>
              <.text as="strong">Source commit</.text>
              <.text as="small" variant={:muted}>{@release["source_ref"]}</.text>
            </.frame>
          </.frame>
          <.action destination={@source_destination}>
            <.text as="code" variant={:mono}>{short_hash(@release["source_commit"])}</.text>
          </.action>
          <.text as="span">Build {short_id(@release["build_request_id"])}</.text>
          <.tag tone={state_tone(@release["build_state"])}>{@release["build_state"]}</.tag>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.frame variant={:resource_primary}>
            <.glyph name="hero-document-text" />
            <.frame variant={:resource_detail}>
              <.text as="strong">Manifest</.text>
              <.text as="small" variant={:muted}>SHA-256</.text>
            </.frame>
          </.frame>
          <.text as="code" variant={:mono}>{short_hash(@release["manifest_hash"])}</.text>
          <.text as="span">Configuration</.text>
          <.text as="code" variant={:mono}>{short_hash(@release["configuration_hash"])}</.text>
        </.frame>
      </.frame>

      <.page_heading
        eyebrow="Immutable release contents"
        title="Imported runtime files"
        description="Files copied from the sealed build output and stored immutably."
        level="h2"
      />
      <.resource_list id="release-artifacts" layout={:projects} update="stream">
        <:header>
          <.text as="span" variant={:sr_only}>Imported runtime files</.text>
        </:header>
        <:empty :if={@release["artifacts"] == []}>No artifacts are visible.</:empty>
        <.frame
          :for={{dom_id, artifact} <- @artifacts}
          as="article"
          id={dom_id}
          variant={:table_row}
        >
          <.frame variant={:resource_primary}>
            <.glyph name="hero-document" />
            <.frame variant={:resource_detail}>
              <.text as="strong">{artifact["path"]}</.text>
              <.text as="small" variant={:muted}>{artifact["media_type"]}</.text>
            </.frame>
          </.frame>
          <.text as="span">{artifact["kind"]}</.text>
          <.text as="code" variant={:mono}>0{Integer.to_string(artifact["mode"], 8)}</.text>
          <.text as="span">
            {format_bytes(artifact["size_bytes"])} · {short_hash(artifact["content_hash"])}
          </.text>
        </.frame>
      </.resource_list>

      <.page_heading
        eyebrow="Project-ready exports"
        title="Runnable agent definitions"
        description="Validated runtime definitions that projects can import as agent instances."
        level="h2"
      />
      <.resource_list id="release-agents" layout={:projects} update="stream">
        <:header>
          <.text as="span" variant={:sr_only}>Runnable agent definitions</.text>
        </:header>
        <:empty :if={@release["agents"] == []}>No exported agents are visible.</:empty>
        <.frame
          :for={{dom_id, agent} <- @agents}
          as="article"
          id={dom_id}
          variant={:table_row}
        >
          <.frame variant={:resource_primary}>
            <.glyph name="hero-cpu-chip" />
            <.frame variant={:resource_detail}>
              <.text as="strong">{agent["display_name"]}</.text>
              <.text as="small" variant={:muted}>{agent["agent_key"]}</.text>
            </.frame>
          </.frame>
          <.text as="span">
            {if(agent["requires_state"], do: "persistent state", else: "stateless")}
          </.text>
          <.text as="span">{length(agent["parameter_schema"])} parameters</.text>
          <.text as="span">{length(agent["secret_slot_schema"])} secret slots</.text>
        </.frame>
      </.resource_list>
    </.frame>
    """
  end

  defp short_id(nil), do: "—"
  defp short_id(value), do: String.slice(value, 0, 8)
  defp short_hash(nil), do: "—"

  defp short_hash(value) when is_binary(value) and byte_size(value) in [20, 32],
    do: value |> Base.encode16(case: :lower) |> String.slice(0, 16)

  defp short_hash(value) when is_binary(value), do: String.slice(value, 0, 16)

  defp format_bytes(bytes) when bytes < 1_024, do: "#{bytes} B"
  defp format_bytes(bytes), do: "#{Float.round(bytes / 1_024, 1)} KiB"

  defp state_tone(value) when value in ["published", "drafted", "succeeded"], do: "success"
  defp state_tone(value) when value in ["revoked", "failed"], do: "danger"
  defp state_tone(_value), do: "neutral"
end
