defmodule HephaestusWebWeb.DesignSystem.Pages.RunPage do
  @moduledoc "Pure presentation for an exact agent run."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]

  attr :state, :atom, default: :loading, values: @states
  attr :run, :map, default: nil
  attr :patch, :string, default: nil
  attr :manifest, :string, default: nil
  attr :events, :any, required: true
  attr :artifacts, :any, required: true
  attr :organization_index_destination, :string, default: nil
  attr :organization_destination, :string, default: nil
  attr :repository_destination, :string, default: nil
  attr :release_destination, :string, default: nil
  attr :agent_destination, :string, default: nil
  attr :control_event, :string, required: true, values: ["control"]

  @doc "Renders run status, review controls, events, and durable artifacts."
  def run(assigns) do
    ~H"""
    <.page_state
      :if={@state != :ready}
      id="run-page-state"
      state={@state}
      title="Run unavailable"
      message="The run is not ready."
    />
    <.frame :if={@state == :ready} variant={:summary_body}>
      <.breadcrumbs id="run-breadcrumbs">
        <:item navigate={@organization_index_destination}>Organizations</:item>
        <:item navigate={@organization_destination}>{@run["organization_name"]}</:item>
        <:item navigate={@repository_destination}>{@run["repository_name"]}</:item>
        <:current>Run {short_sha(@run["id"])}</:current>
      </.breadcrumbs>

      <.frame as="section" variant={:run_hero}>
        <.frame variant={:summary_body}>
          <.frame variant={:run_title}>
            <.tag tone={run_tone(@run)} dot>{human_state(@run["outcome"] || @run["state"])}</.tag>
            <.frame variant={:resource_detail}>
              <.text as="p" variant={:eyebrow}>Agent run · attempt {@run["attempt"]}</.text>
              <.text as="h1" variant={:title}>{@run["agent_name"]}</.text>
            </.frame>
          </.frame>
          <.text as="p" variant={:lede}>
            Exact input
            <.text as="code" variant={:mono}>{short_sha(@run["input_commit"])}</.text>
            on
            <.text as="code" variant={:mono}>{@run["git_ref"]}</.text>
          </.text>
          <.frame id="run-exact-provenance" variant={:command_row}>
            <.action destination={@release_destination}>Release {@run["release_version"]}</.action>
            <.text as="span">·</.text>
            <.action destination={@agent_destination}>
              Revision {short_sha(@run["instance_revision_id"])}
            </.action>
            <.text as="span">·</.text>
            <.text as="code" variant={:mono}>target {short_sha(@run["input_commit"])}</.text>
          </.frame>
        </.frame>
        <.frame variant={:control_bar}>
          <.action
            :if={active?(@run)}
            interaction={:event}
            event={@control_event}
            event_payload={%{kind: "cancel_run"}}
            variant={:secondary}
            test_id="cancel-run"
          >
            Cancel
          </.action>
          <.action
            interaction={:event}
            event={@control_event}
            event_payload={%{kind: "retry_run"}}
            variant={:secondary}
            test_id="retry-run"
          >
            Retry exact input
          </.action>
        </.frame>
      </.frame>

      <.frame variant={:metric_grid}>
        <.metric label="State" value={human_state(@run["outcome"] || @run["state"])} />
        <.metric label="Duration" value={"#{@run["metrics"]["elapsed_ms"]} ms"} />
        <.metric label="Events" value={@run["metrics"]["event_count"]} />
        <.metric label="Log frames" value={@run["metrics"]["log_count"]} />
      </.frame>

      <.frame
        :if={@run["runtime_metrics"] != []}
        as="section"
        id="runtime-metrics"
        variant={:panel}
        test_id="runtime-metrics"
      >
        <.frame variant={:summary_header}>
          <.page_heading eyebrow="Guest telemetry" title="Latest samples" level="h2" />
          <.tag tone="success" dot>persisted</.tag>
        </.frame>
        <.frame variant={:metric_grid}>
          <.metric
            :for={metric <- @run["runtime_metrics"]}
            label={metric["name"]}
            value={metric_value(metric)}
          />
        </.frame>
      </.frame>

      <.proposal :if={@run["proposal_id"]} run={@run} event={@control_event} />

      <.frame variant={:review_grid}>
        <.frame as="section" variant={:artifact_panel}>
          <.frame variant={:summary_header}>
            <.page_heading eyebrow="Workspace result" title="Patch" level="h2" />
            <.tag>{artifact_size(@run, "patch")}</.tag>
          </.frame>
          <.text as="pre" id="result-diff" variant={:mono} test_id="result-diff">
            {@patch || "No result patch has been published yet."}
          </.text>
        </.frame>

        <.frame as="section" variant={:timeline_panel}>
          <.frame variant={:summary_header}>
            <.page_heading eyebrow="Persisted lifecycle" title="Timeline" level="h2" />
            <.tag tone="success" dot>live</.tag>
          </.frame>
          <.frame
            as="ol"
            id="run-timeline"
            variant={:timeline}
            test_id="run-timeline"
            phx_update="stream"
          >
            <.frame :for={{dom_id, event} <- @events} as="li" id={dom_id} variant={:timeline_item}>
              <.frame variant={:timeline_dot} />
              <.frame variant={:resource_detail}>
                <.text as="strong">{event_label(event["event_type"])}</.text>
                <.text as="small" variant={:muted}>
                  #{event["sequence"]} · {Calendar.strftime(event["occurred_at"], "%H:%M:%S")}
                </.text>
                <.text
                  :if={event["event_type"] == "vm.log"}
                  as="code"
                  variant={:mono}
                >
                  {log_line(event["payload"])}
                </.text>
              </.frame>
            </.frame>
          </.frame>
        </.frame>
      </.frame>

      <.frame as="section" variant={:artifact_panel}>
        <.frame variant={:summary_header}>
          <.page_heading eyebrow="Durable provenance" title="Artifacts" level="h2" />
          <.text as="code" variant={:mono}>{short_sha(@run["artifact_manifest_hash"])}</.text>
        </.frame>
        <.frame id="artifacts" variant={:artifact_list} phx_update="stream">
          <.frame
            :for={{dom_id, artifact} <- @artifacts}
            id={dom_id}
            variant={:artifact_row}
          >
            <.tag tone="accent">{artifact["kind"]}</.tag>
            <.text as="span">{artifact["path"] || "run output"}</.text>
            <.text as="code" variant={:mono}>{format_bytes(artifact["size_bytes"])}</.text>
            <.text as="code" variant={:mono}>{short_sha(artifact["sha256"])}</.text>
          </.frame>
        </.frame>
        <.frame :if={@manifest} as="details" variant={:summary_body}>
          <.frame as="summary" variant={:tree_summary}>Inspect manifest JSON</.frame>
          <.text as="pre" variant={:mono}>{@manifest}</.text>
        </.frame>
      </.frame>
    </.frame>
    """
  end

  attr :run, :map, required: true
  attr :event, :string, required: true, values: ["control"]

  defp proposal(assigns) do
    ~H"""
    <.frame as="section" variant={:proposal} test_id="review-proposal">
      <.frame variant={:proposal_heading}>
        <.page_heading
          eyebrow="Controlled result proposal"
          title={@run["target_ref"]}
          level="h2"
        />
        <.tag tone={proposal_tone(@run["proposal_state"])}>{@run["proposal_state"]}</.tag>
      </.frame>
      <.frame variant={:commit_flow}>
        <.metric label="Input" value={short_sha(@run["input_commit"])} />
        <.text as="span">→</.text>
        <.metric label="Result" value={short_sha(@run["result_commit"])} />
        <.text as="span">→</.text>
        <.metric label="Target" value={@run["target_ref"]} />
      </.frame>
      <.text as="p" variant={:result_message}>“{@run["result_message"]}”</.text>
      <.frame
        :if={@run["proposal_state"] in ["open", "approval_requested"]}
        variant={:review_actions}
      >
        <.action
          interaction={:event}
          event={@event}
          event_payload={%{kind: "reject_result"}}
          variant={:danger}
          test_id="reject-result"
        >
          Reject
        </.action>
        <.action
          interaction={:event}
          event={@event}
          event_payload={%{kind: "approve_result"}}
          variant={:primary}
          test_id="approve-result"
        >
          Approve fast-forward
        </.action>
      </.frame>
    </.frame>
    """
  end

  attr :label, :string, required: true
  attr :value, :any, required: true

  defp metric(assigns) do
    ~H"""
    <.frame variant={:metric}>
      <.text as="small" variant={:muted}>{@label}</.text>
      <.text as="strong">{@value}</.text>
    </.frame>
    """
  end

  defp artifact_size(run, kind) do
    case Enum.find(run["artifacts"], &(&1["kind"] == kind)) do
      nil -> "pending"
      artifact -> format_bytes(artifact["size_bytes"])
    end
  end

  defp short_sha(nil), do: "pending"
  defp short_sha(value), do: String.slice(value, 0, 10)
  defp human_state(value), do: value |> String.replace("_", " ") |> String.capitalize()
  defp active?(run), do: run["state"] in ~w(queued leasing_volume provisioning starting running)

  defp run_tone(%{"outcome" => "succeeded"}), do: "success"
  defp run_tone(%{"outcome" => outcome}) when outcome in ["failed", "cancelled"], do: "danger"
  defp run_tone(_run), do: "warning"

  defp proposal_tone("approved"), do: "success"
  defp proposal_tone(state) when state in ["rejected", "conflicted"], do: "danger"
  defp proposal_tone(_state), do: "warning"
  defp event_label(value), do: value |> String.replace(".", " · ") |> String.replace("_", " ")

  defp log_line(payload) do
    payload["line"] || payload["message"] || payload["data"] || Jason.encode!(payload)
  end

  defp format_bytes(nil), do: "—"
  defp format_bytes(value) when value < 1_024, do: "#{value} B"
  defp format_bytes(value) when value < 1_048_576, do: "#{Float.round(value / 1_024, 1)} KB"
  defp format_bytes(value), do: "#{Float.round(value / 1_048_576, 1)} MB"

  defp metric_value(metric) do
    labels =
      metric["labels"]
      |> Enum.sort()
      |> Enum.map_join(" ", fn {key, value} -> "#{key}=#{value}" end)

    if labels == "", do: metric["value"], else: "#{metric["value"]} · #{labels}"
  end
end
