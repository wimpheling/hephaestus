defmodule HephaestusWebWeb.DesignSystem.Pages.BuildPage do
  @moduledoc "Pure presentation for a typed build detail."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :build, :map, default: nil
  attr :repository, :map, default: nil
  attr :logs, :list, required: true
  attr :metrics, :list, required: true
  attr :organization_index_destination, :string, default: nil
  attr :organization_destination, :string, default: nil
  attr :project_destination, :string, default: nil
  attr :repository_destination, :string, default: nil
  attr :release_destination, :string, default: nil
  attr :timeline, :list, default: []
  attr :declared_artifacts, :list, default: []
  attr :produced_artifacts, :list, default: []
  attr :artifact_manifest, :any, default: []
  attr :retry_event, :string, default: nil
  attr :verification_rebuild_event, :string, default: nil
  attr :another_commit_event, :string, default: nil

  def build(assigns) do
    ~H"""
    <.page_state
      :if={@state != :ready}
      id="build-page-state"
      state={@state}
      title="Build unavailable"
      message="The build is not ready. Live updates resume from the committed repository cursor when the connection returns."
    />
    <.frame :if={@state == :ready} variant={:summary_body}>
      <.breadcrumbs id="build-breadcrumbs">
        <:item navigate={@organization_index_destination}>Organizations</:item>
        <:item navigate={@organization_destination}>{@repository["organization_name"]}</:item>
        <:item navigate={@project_destination}>{@repository["project_name"]}</:item>
        <:item navigate={@repository_destination}>{@repository["name"]}</:item>
        <:current>Build {short_id(@build["id"])}</:current>
      </.breadcrumbs>

      <.page_heading
        eyebrow="Agent release build"
        title={short_id(@build["id"])}
        description="Typed GetBuild data for one immutable build identity."
      >
        <:actions>
          <.tag tone={state_tone(@build["state"])} dot>{@build["state"]}</.tag>
        </:actions>
      </.page_heading>

      <.frame as="section" id="build-provenance" variant={:table}>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Build identity</.text><.text as="code" variant={:mono}>
            {@build["id"]}
          </.text>
          <.text as="strong">Exit status</.text><.text as="span">{exit_status(@build)}</.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Created</.text><.text as="time">
            {display_time(@build["created_at"])}
          </.text>
          <.text as="strong">Updated</.text><.text as="time">
            {display_time(@build["updated_at"])}
          </.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Failure diagnostic</.text><.text as="span">
            {@build["failure_code"] || "—"}
          </.text>
          <.text as="strong">Source commit</.text>
          <.text as="code" variant={:mono}>{@build["source_commit"] || "—"}</.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Source ref</.text><.text as="code" variant={:mono}>
            {@build["source_ref"] || "—"}
          </.text>
          <.text as="strong">Build definition</.text><.text as="code" variant={:mono}>
            {@build["build_definition_hash"] || "—"}
          </.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Trigger</.text><.text as="span">{@build["trigger"] || "—"}</.text>
          <.text as="strong">Agent key</.text><.text as="code" variant={:mono}>
            {@build["agent_key"] || "—"}
          </.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Builder</.text><.text as="code" variant={:mono}>
            {@build["builder_image_key"] || @build["builder_image_reference"] || "—"}
          </.text>
          <.text as="strong">Builder digest</.text><.text as="code" variant={:mono}>
            {@build["builder_image_reference"] || "—"}
          </.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Configuration hash</.text><.text as="code" variant={:mono}>
            {@build["configuration_hash"] || "—"}
          </.text>
          <.text as="strong">Draft version</.text><.text as="span">
            {@build["release_version"] || "—"}
          </.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Started</.text><.text as="time">
            {display_time(@build["started_at"])}
          </.text>
          <.text as="strong">Duration</.text><.text as="span">
            {display_duration(@build["duration_milliseconds"])}
          </.text>
        </.frame>
        <.frame as="article" variant={:table_row}>
          <.text as="strong">Release state</.text><.text as="span">
            {@build["release_state"] || "Not released"}
          </.text>
          <.text as="strong">Artifacts</.text><.text as="span">
            {Integer.to_string(@build["artifact_count"] || 0)}
          </.text>
        </.frame>
      </.frame>

      <.frame as="section" id="build-metrics" variant={:metric_grid}>
        <.metric :for={metric <- @metrics} label={metric["name"]} value={metric_value(metric)} />
        <.metric :if={@metrics == []} label="Runtime metrics" value="Not supplied" />
      </.frame>

      <.frame as="section" id="build-logs" variant={:artifact_panel}>
        <.frame variant={:summary_header}>
          <.page_heading eyebrow="Bounded output" title="Logs" level="h2" />
          <.tag tone="neutral">GetBuild</.tag>
        </.frame>
        <.text :if={@logs == []} as="p" variant={:muted}>No logs were returned.</.text>
        <.text :for={line <- @logs} as="pre" variant={:mono}>{line}</.text>
        <.text as="small" variant={:muted}>
          Log reconnect and truncation metadata are not supplied by GetBuild.
        </.text>
      </.frame>

      <.frame as="section" id="build-declaration" variant={:review_grid}>
        <.frame variant={:panel}>
          <.page_heading eyebrow="Build declaration" title="Configuration" level="h2" />
          <.text as="pre" variant={:mono}>{inspect_value(@build["parsed_declaration"])}</.text>
          <.text as="small" variant={:muted}>
            The declaration is the parsed, secret-free build input frozen at request time.
          </.text>
        </.frame>
        <.frame variant={:panel}>
          <.page_heading eyebrow="Execution policy" title="Resources and network" level="h2" />
          <.text as="pre" variant={:mono}>{inspect_value(@build["build_policy"])}</.text>
          <.text as="small" variant={:muted}>
            Policy is displayed from the durable build snapshot, not from the current repository.
          </.text>
        </.frame>
      </.frame>

      <.frame as="section" id="build-timeline" variant={:timeline_panel}>
        <.page_heading eyebrow="Durable lifecycle" title="State timeline" level="h2" />
        <.text :if={@timeline == []} as="p" variant={:muted}>
          No timeline entries were returned.
        </.text>
        <.frame
          :for={{event, index} <- Enum.with_index(@timeline)}
          as="article"
          id={"build-transition-#{index}"}
          variant={:timeline_item}
        >
          <.frame variant={:timeline_dot} />
          <.frame variant={:panel}>
            <.text as="strong">{event["to_state"] || "unknown"}</.text>
            <.text as="span" variant={:muted}>{event["reason"] || "state changed"}</.text>
            <.text as="time" variant={:muted}>{display_time(event["occurred_at"])}</.text>
          </.frame>
        </.frame>
      </.frame>

      <.frame as="section" id="build-artifacts" variant={:review_grid}>
        <.frame variant={:panel}>
          <.page_heading eyebrow="Declared outputs" title="Build contract" level="h2" />
          <.text :if={@declared_artifacts == []} as="p" variant={:muted}>
            No declared artifacts were returned.
          </.text>
          <.frame :for={artifact <- @declared_artifacts} variant={:table_row}>
            <.text as="code" variant={:mono}>{artifact["path"]}</.text>
            <.text as="span">{artifact["kind"]}</.text>
          </.frame>
        </.frame>
        <.frame variant={:panel}>
          <.page_heading eyebrow="Imported outputs" title="Produced artifacts" level="h2" />
          <.text :if={@produced_artifacts == []} as="p" variant={:muted}>
            No produced artifacts were returned.
          </.text>
          <.frame :for={artifact <- @produced_artifacts} variant={:table_row}>
            <.text as="code" variant={:mono}>{artifact["path"]}</.text>
            <.text as="span">{artifact["size_bytes"]} bytes</.text>
            <.text as="code" variant={:mono}>{artifact["sha256"]}</.text>
          </.frame>
        </.frame>
      </.frame>

      <.frame as="section" id="build-manifest" variant={:panel}>
        <.page_heading eyebrow="Imported manifest" title="Manifest snapshot" level="h2" />
        <.text as="pre" variant={:mono}>{inspect_value(@artifact_manifest)}</.text>
      </.frame>

      <.frame as="section" id="build-release" variant={:panel}>
        <.frame variant={:panel}>
          <.page_heading eyebrow="Draft release relation" title="Release result" level="h2" />
          <.text as="p" variant={:muted}>
            The build projection retains the release identity and state created by artifact import.
          </.text>
          <.action :if={@release_destination} destination={@release_destination}>
            Open draft release
          </.action>
        </.frame>
      </.frame>

      <.frame as="section" id="build-actions" variant={:panel}>
        <.page_heading eyebrow="Build controls" title="Distinct actions" level="h2" />
        <.frame variant={:resource_controls}>
          <.action
            :if={@retry_event}
            interaction={:event}
            event={@retry_event}
            variant={:secondary}
            disable_with="Queueing retry…"
          >
            Retry attempt
          </.action>
          <.action
            :if={@verification_rebuild_event}
            interaction={:event}
            event={@verification_rebuild_event}
            variant={:secondary}
            confirm="Run the immutable build inputs again for verification?"
            disable_with="Queueing verification…"
          >
            Rebuild for verification
          </.action>
          <.text :if={!@retry_event && !@verification_rebuild_event} as="span" variant={:muted}>
            No lifecycle action is available for the current build state.
          </.text>
          <.text as="span">
            Build another commit — request a build for a different source commit.
          </.text>
        </.frame>
      </.frame>
    </.frame>
    """
  end

  defp short_id(nil), do: "—"
  defp short_id(value), do: String.slice(value, 0, 12)

  defp exit_status(%{"exit_code" => exit_code}) when is_integer(exit_code),
    do: Integer.to_string(exit_code)

  defp exit_status(_build), do: "—"
  defp state_tone("succeeded"), do: "success"
  defp state_tone("failed"), do: "danger"
  defp state_tone("running"), do: "warning"
  defp state_tone(_state), do: "neutral"
  defp metric_value(metric), do: "#{metric["value"]} #{metric["unit"]}"

  defp metric(assigns) do
    ~H"""
    <.frame variant={:metric}>
      <.text as="small" variant={:muted}>{@label}</.text>
      <.text as="strong">{@value}</.text>
    </.frame>
    """
  end

  defp display_time(%DateTime{} = value), do: Calendar.strftime(value, "%d %b %Y · %H:%M:%S UTC")
  defp display_time(nil), do: "—"
  defp display_time(value), do: to_string(value)

  defp display_duration(value) when is_integer(value), do: "#{value} ms"
  defp display_duration(value) when is_float(value), do: "#{Float.round(value, 1)} ms"
  defp display_duration(_value), do: "—"

  defp inspect_value(nil), do: "—"
  defp inspect_value(value), do: inspect(value, pretty: true, limit: 100)
end
