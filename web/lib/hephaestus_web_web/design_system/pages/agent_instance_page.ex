defmodule HephaestusWebWeb.DesignSystem.Pages.AgentInstancePage do
  @moduledoc "Pure presentation for a project-owned agent instance."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]

  attr :state, :atom, default: :loading, values: @states
  attr :instance, :map, default: nil
  attr :revisions, :any, required: true
  attr :attachments, :any, required: true
  attr :updates, :any, required: true
  attr :attachment_form, :any, required: true
  attr :revision_form, :any, required: true
  attr :update_form, :any, required: true
  attr :binding_form, :any, required: true
  attr :organization_index_destination, :string, default: nil
  attr :organization_destination, :string, default: nil
  attr :project_agents_destination, :string, default: nil
  attr :repositories_tab_destination, :string, default: nil
  attr :agents_tab_destination, :string, default: nil
  attr :runs_tab_destination, :string, default: nil
  attr :settings_tab_destination, :string, default: nil
  attr :run_destination, :any, required: true
  attr :create_attachment_event, :string, required: true, values: ["create-attachment"]
  attr :set_attachment_event, :string, required: true, values: ["set-attachment"]
  attr :remove_attachment_event, :string, required: true, values: ["remove-attachment"]
  attr :revise_instance_event, :string, required: true, values: ["revise-instance"]
  attr :create_update_event, :string, required: true, values: ["create-update"]
  attr :recover_update_event, :string, required: true, values: ["recover-update"]
  attr :bind_secret_event, :string, required: true, values: ["bind-secret"]

  @doc "Renders an agent instance from route-provided presentation data."
  def agent_instance(assigns) do
    assigns =
      assign(assigns, :tabs, [
        %{
          key: :repositories,
          label: "Repositories",
          icon: "hero-circle-stack",
          destination: assigns.repositories_tab_destination
        },
        %{
          key: :agents,
          label: "Agents",
          icon: "hero-cpu-chip",
          destination: assigns.agents_tab_destination
        },
        %{
          key: :runs,
          label: "Runs",
          icon: "hero-play-circle",
          destination: assigns.runs_tab_destination
        },
        %{
          key: :settings,
          label: "Settings",
          icon: "hero-cog-6-tooth",
          destination: assigns.settings_tab_destination
        }
      ])

    ~H"""
    <.page_state
      :if={@state != :ready}
      id="agent-instance-page-state"
      state={@state}
      title="Agent instance unavailable"
      message="The agent instance is not ready."
    />
    <.frame :if={@state == :ready} variant={:summary_body}>
      <.breadcrumbs id="instance-breadcrumbs">
        <:item navigate={@organization_index_destination}>Organizations</:item>
        <:item navigate={@organization_destination}>{@instance["organization_name"]}</:item>
        <:item navigate={@project_agents_destination}>{@instance["project_name"]}</:item>
        <:current>{@instance["name"]}</:current>
      </.breadcrumbs>

      <.page_heading
        eyebrow="Project agent instance"
        title={@instance["name"]}
        description="Stable installation identity with immutable revisions and exact release provenance."
      >
        <:actions>
          <.tag tone={state_tone(@instance["state"])}>{@instance["state"]}</.tag>
          <.tag tone={if(@instance["run_gate_open"], do: "success", else: "warning")}>
            {if(@instance["run_gate_open"], do: "run gate open", else: "run gate closed")}
          </.tag>
        </:actions>
      </.page_heading>

      <.tab_navigation id="project-tabs" label="Project" items={@tabs} active={:agents} />

      <.overview instance={@instance} />
      <.management
        :if={@instance["can_manage"]}
        instance={@instance}
        revision_form={@revision_form}
        binding_form={@binding_form}
        revise_event={@revise_instance_event}
        bind_event={@bind_secret_event}
      />
      <.revision_history revisions={@revisions} />
      <.attachment_section
        instance={@instance}
        attachments={@attachments}
        form={@attachment_form}
        create_event={@create_attachment_event}
        set_event={@set_attachment_event}
        remove_event={@remove_attachment_event}
      />
      <.update_section
        instance={@instance}
        updates={@updates}
        form={@update_form}
        create_event={@create_update_event}
        recover_event={@recover_update_event}
      />
      <.recent_runs
        runs={@instance["recent_runs"]}
        destination={@run_destination}
      />
    </.frame>
    """
  end

  attr :instance, :map, required: true

  defp overview(assigns) do
    ~H"""
    <.frame as="section" id="instance-overview" variant={:instance_overview}>
      <.frame as="article" variant={:metric}>
        <.text as="small" variant={:muted}>Active revision</.text>
        <.text as="strong">{short_id(@instance["active_revision_id"])}</.text>
      </.frame>
      <.frame as="article" variant={:metric}>
        <.text as="small" variant={:muted}>State volume</.text>
        <.text as="strong">
          {if(@instance["state_volume_id"], do: "healthy · retained", else: "stateless")}
        </.text>
      </.frame>
      <.frame as="article" variant={:metric}>
        <.text as="small" variant={:muted}>Recent runs</.text>
        <.text as="strong">{length(@instance["recent_runs"])}</.text>
      </.frame>
      <.frame as="article" variant={:metric}>
        <.text as="small" variant={:muted}>Update availability</.text>
        <.text as="strong">{length(@instance["update_candidates"])} compatible releases</.text>
      </.frame>
    </.frame>
    """
  end

  attr :instance, :map, required: true
  attr :revision_form, :any, required: true
  attr :binding_form, :any, required: true
  attr :revise_event, :string, required: true, values: ["revise-instance"]
  attr :bind_event, :string, required: true, values: ["bind-secret"]

  defp management(assigns) do
    ~H"""
    <.frame as="section" variant={:two_column}>
      <.revision_form instance={@instance} form={@revision_form} event={@revise_event} />
      <.binding_forms instance={@instance} form={@binding_form} event={@bind_event} />
    </.frame>
    """
  end

  attr :instance, :map, required: true
  attr :form, :any, required: true
  attr :event, :string, required: true, values: ["revise-instance"]

  defp revision_form(assigns) do
    ~H"""
    <.frame as="article" id="revise-instance-panel" variant={:panel}>
      <.text as="h2" variant={:title}>New parameter revision</.text>
      <.text as="p">
        Review old and candidate values. Sensitive parameters are never redisplayed and must be entered again.
      </.text>
      <.form_container for={@form} id="revise-instance" submit={@event}>
        <.input
          :for={declaration <- active_revision(@instance)["parameter_schema"]}
          id={"revision-parameter-#{declaration["name"]}"}
          name={"revision[parameters][#{declaration["name"]}]"}
          label={parameter_label(declaration, active_parameter(@instance, declaration))}
          type={parameter_input_type(declaration)}
          options={parameter_options(declaration)}
          value={active_parameter(@instance, declaration)}
          required={declaration["required"]}
          autocomplete="off"
        />
        <.input
          field={@form[:vcpus]}
          type="number"
          label="Virtual CPUs"
          value={active_policy(@instance, "vcpus")}
          min="1"
          required
        />
        <.input
          field={@form[:memory_mib]}
          type="number"
          label="Memory (MiB)"
          value={active_policy(@instance, "memory_mib")}
          min="1"
          required
        />
        <.input
          field={@form[:network]}
          type="select"
          label="Network restriction"
          value={active_policy(@instance, "network")}
          options={network_options()}
        />
        <.action interaction={:submit} variant={:primary}>Review and activate revision</.action>
      </.form_container>
    </.frame>
    """
  end

  attr :instance, :map, required: true
  attr :form, :any, required: true
  attr :event, :string, required: true, values: ["bind-secret"]

  defp binding_forms(assigns) do
    ~H"""
    <.frame as="article" id="secret-binding-panel" variant={:panel}>
      <.text as="h2" variant={:title}>Bind declared secret slot</.text>
      <.text as="p">
        Only live imports eligible for this project or an enabled exact attachment are listed.
      </.text>
      <.text
        :if={active_revision(@instance)["secret_slot_schema"] == []}
        as="p"
        variant={:empty}
      >
        This release declares no secret slots.
      </.text>
      <.form_container
        :for={slot <- active_revision(@instance)["secret_slot_schema"]}
        for={@form}
        id={"bind-secret-#{slot["key"]}"}
        submit={@event}
      >
        <.frame variant={:binding_form}>
          <.input type="hidden" name="binding[slot]" value={slot["key"]} />
          <.frame as="header" variant={:summary_header}>
            <.text as="strong">{slot["key"]}</.text>
            <.tag tone={if(slot["required"], do: "warning", else: "neutral")}>
              {if(slot["required"], do: "required", else: "optional")}
            </.tag>
          </.frame>
          <.text as="p">{slot["purpose"]}</.text>
          <.input
            id={"binding-import-#{slot["key"]}"}
            name="binding[import_id]"
            type="select"
            label="Eligible opaque import"
            prompt="Choose an import"
            options={import_options(@instance["secret_imports"], slot)}
            value=""
            required
          />
          <.input
            id={"binding-mode-#{slot["key"]}"}
            name="binding[mode]"
            type="select"
            label="Delivery mode"
            options={mode_options(slot)}
            value={List.first(slot["delivery_modes"])}
            required
          />
          <.input
            id={"binding-phases-#{slot["key"]}"}
            name="binding[phases][]"
            type="select"
            label="Phases"
            options={Enum.map(slot["phases"], &{human_phase(&1), &1})}
            value={slot["phases"]}
            multiple
            required
          />
          <.input
            id={"binding-attachments-#{slot["key"]}"}
            name="binding[attachment_ids][]"
            type="select"
            label="Exact attachments"
            options={attachment_options(@instance["attachments"])}
            value={[]}
            multiple
          />
          <.input
            id={"binding-destinations-#{slot["key"]}"}
            name="binding[destinations]"
            label="Broker destinations"
            value={Enum.join(slot["destinations"], ", ")}
            readonly={slot["destinations"] != []}
          />
          <.frame variant={:confirmation}>
            <.input
              id={"binding-raw-confirmation-#{slot["key"]}"}
              name="binding[raw_confirmation]"
              type="checkbox"
              label="I understand raw mode gives the guest plaintext it can copy."
              value="false"
            />
          </.frame>
          <.action interaction={:submit} variant={:primary}>Create binding revision</.action>
        </.frame>
      </.form_container>
    </.frame>
    """
  end

  attr :revisions, :any, required: true

  defp revision_history(assigns) do
    ~H"""
    <.page_heading eyebrow="Immutable history" title="Revisions" level="h2" />
    <.resource_list id="instance-revisions" layout={:projects} update="stream">
      <:header>
        <.text as="span" variant={:sr_only}>Revision history</.text>
      </:header>
      <:empty>No visible revisions.</:empty>
      <.frame
        :for={{dom_id, revision} <- @revisions}
        as="article"
        id={dom_id}
        variant={:table_row}
      >
        <.frame variant={:resource_primary}>
          <.glyph name="hero-numbered-list" />
          <.frame variant={:resource_detail}>
            <.text as="strong">{revision["release_agent_name"]}</.text>
            <.text as="small" variant={:muted}>{short_id(revision["id"])}</.text>
          </.frame>
        </.frame>
        <.tag tone={state_tone(revision["release_state"])}>{revision["release_version"]}</.tag>
        <.text as="span">{revision["platform_policy_version"]}</.text>
        <.tag tone={if(revision["runnable"], do: "success", else: "danger")}>
          {if(revision["runnable"], do: "runnable", else: "invalid")}
        </.tag>
      </.frame>
    </.resource_list>
    """
  end

  attr :instance, :map, required: true
  attr :attachments, :any, required: true
  attr :form, :any, required: true
  attr :create_event, :string, required: true, values: ["create-attachment"]
  attr :set_event, :string, required: true, values: ["set-attachment"]
  attr :remove_event, :string, required: true, values: ["remove-attachment"]

  defp attachment_section(assigns) do
    ~H"""
    <.page_heading eyebrow="Exact targets" title="Attachments" level="h2" />
    <.frame
      :if={@instance["can_manage"]}
      as="article"
      id="create-attachment-panel"
      variant={:panel}
    >
      <.text as="h3" variant={:title}>Attach an exact target</.text>
      <.form_container for={@form} id="create-attachment" submit={@create_event}>
        <.frame variant={:command_grid}>
          <.input
            field={@form[:repository_id]}
            type="select"
            label="Repository"
            prompt="Choose a repository"
            options={Enum.map(@instance["repositories"], &{&1["name"], &1["id"]})}
            required
          />
          <.input
            field={@form[:ref_selector]}
            label="Exact ref or prefix"
            value="refs/heads/main"
            required
          />
          <.input
            field={@form[:trigger_policy]}
            type="select"
            label="Trigger policy"
            value="push"
            options={[
              {"Push", "push"},
              {"Manual", "manual"},
              {"Push and manual", "push_and_manual"}
            ]}
          />
          <.action interaction={:submit} variant={:primary}>Create attachment</.action>
        </.frame>
      </.form_container>
    </.frame>
    <.resource_list id="instance-attachments" layout={:projects} update="stream">
      <:header>
        <.text as="span" variant={:sr_only}>Attachments</.text>
      </:header>
      <:empty>This instance has no attachments.</:empty>
      <.frame
        :for={{dom_id, attachment} <- @attachments}
        as="article"
        id={dom_id}
        variant={:table_row}
      >
        <.frame variant={:resource_primary}>
          <.glyph name="hero-link" />
          <.frame variant={:resource_detail}>
            <.text as="strong">{attachment["repository_name"]}</.text>
            <.text as="small" variant={:muted}>{attachment["trigger_policy"]}</.text>
          </.frame>
        </.frame>
        <.text as="code" variant={:mono}>{attachment["ref_selector"]}</.text>
        <.tag tone={attachment_tone(attachment)}>{attachment_state(attachment)}</.tag>
        <.frame variant={:command_row}>
          <.action
            :if={attachment["can_manage"] && is_nil(attachment["removed_at"])}
            interaction={:event}
            event={@set_event}
            event_payload={
              %{
                attachment_id: attachment["id"],
                enabled: to_string(!attachment["enabled"])
              }
            }
            variant={:compact}
          >
            {if(attachment["enabled"], do: "Disable", else: "Enable")}
          </.action>
          <.action
            :if={attachment["can_manage"] && is_nil(attachment["removed_at"])}
            interaction={:event}
            event={@remove_event}
            event_payload={%{attachment_id: attachment["id"]}}
            confirm="Remove this attachment while retaining historical run provenance?"
            variant={:danger}
          >
            Remove
          </.action>
        </.frame>
      </.frame>
    </.resource_list>
    """
  end

  attr :instance, :map, required: true
  attr :updates, :any, required: true
  attr :form, :any, required: true
  attr :create_event, :string, required: true, values: ["create-update"]
  attr :recover_event, :string, required: true, values: ["recover-update"]

  defp update_section(assigns) do
    ~H"""
    <.page_heading eyebrow="Migration lifecycle" title="Updates" level="h2" />
    <.frame
      :if={@instance["can_update"] && @instance["update_candidates"] != []}
      as="article"
      id="create-update-panel"
      variant={:panel}
    >
      <.text as="h3" variant={:title}>Review a release update</.text>
      <.text as="p">
        Release-owned runtime changes are shown as immutable candidate provenance. Unsupported state-capability changes and hookless stateful migrations remain invalid with stable diagnostics.
      </.text>
      <.form_container for={@form} id="create-update" submit={@create_event}>
        <.input
          field={@form[:release_agent_id]}
          type="select"
          label="Candidate release"
          prompt="Choose a compatible release"
          options={candidate_options(@instance["update_candidates"])}
          required
        />
        <.input
          :for={declaration <- candidate_schema(@instance)}
          id={"update-parameter-#{declaration["name"]}"}
          name={"update[parameters][#{declaration["name"]}]"}
          label={parameter_label(declaration, nil)}
          type={parameter_input_type(declaration)}
          options={parameter_options(declaration)}
          value={candidate_default(@instance, declaration)}
          required={declaration["required"]}
          autocomplete="off"
        />
        <.input
          field={@form[:vcpus]}
          type="number"
          label="Candidate virtual CPUs"
          value={candidate_policy(@instance, "vcpus")}
          min="1"
          required
        />
        <.input
          field={@form[:memory_mib]}
          type="number"
          label="Candidate memory (MiB)"
          value={candidate_policy(@instance, "memory_mib")}
          min="1"
          required
        />
        <.input
          field={@form[:network]}
          type="select"
          label="Candidate network restriction"
          value={candidate_policy(@instance, "network")}
          options={network_options()}
        />
        <.action interaction={:submit} variant={:primary}>Start reviewed update</.action>
      </.form_container>
    </.frame>
    <.resource_list id="instance-updates" layout={:projects} update="stream">
      <:header>
        <.text as="span" variant={:sr_only}>Release updates</.text>
      </:header>
      <:empty>No release updates have been attempted.</:empty>
      <.frame
        :for={{dom_id, update} <- @updates}
        as="article"
        id={dom_id}
        variant={:table_row}
      >
        <.frame variant={:resource_primary}>
          <.glyph name="hero-arrow-path" />
          <.frame variant={:resource_detail}>
            <.text as="strong">{update["state"]}</.text>
            <.text as="small" variant={:muted}>{short_id(update["id"])}</.text>
          </.frame>
        </.frame>
        <.text as="span">{short_id(update["expected_current_revision_id"])}</.text>
        <.text as="span">{short_id(update["candidate_revision_id"])}</.text>
        <.frame variant={:resource_detail}>
          <.tag tone={state_tone(update["state"])}>{update["final_decision"] || "pending"}</.tag>
          <.frame
            :if={update["hook_events"] != []}
            id={"update-hook-events-#{update["id"]}"}
            variant={:timeline}
            aria_live="polite"
            aria_label="Update hook events"
            tabindex={0}
          >
            <.text :for={event <- update["hook_events"]} as="small" variant={:muted}>
              {event["sequence"]} · {event["event_type"]} · {hook_event_summary(event)}
            </.text>
          </.frame>
          <.frame
            :if={@instance["can_recover"] && recovery_actions(update) != []}
            variant={:command_row}
          >
            <.action
              :for={{label, action} <- recovery_actions(update)}
              id={"recover-#{action}-#{update["id"]}"}
              interaction={:event}
              event={@recover_event}
              event_payload={%{update_id: update["id"], action: action}}
              confirm={"Apply authorized recovery action: #{label}?"}
              variant={:compact}
            >
              {label}
            </.action>
          </.frame>
        </.frame>
      </.frame>
    </.resource_list>
    """
  end

  attr :runs, :list, required: true
  attr :destination, :any, required: true

  defp recent_runs(assigns) do
    ~H"""
    <.page_heading eyebrow="Exact provenance" title="Recent runs" level="h2" />
    <.resource_list id="instance-recent-runs" layout={:projects}>
      <:header>
        <.text as="span" variant={:sr_only}>Recent runs</.text>
      </:header>
      <:empty :if={@runs == []}>No runs yet.</:empty>
      <:row :for={run <- @runs}>
        <.action id={"instance-run-#{run["id"]}"} destination={@destination.(run["id"])}>
          <.frame variant={:table_row}>
            <.text as="span">{short_id(run["id"])}</.text>
            <.text as="span">{run["run_kind"]}</.text>
            <.text as="span">{short_id(run["instance_revision_id"])}</.text>
            <.tag tone={state_tone(run["outcome"] || run["state"])}>
              {run["outcome"] || run["state"]}
            </.tag>
          </.frame>
        </.action>
      </:row>
    </.resource_list>
    """
  end

  defp active_revision(instance) do
    Enum.find(instance["revisions"], &(&1["id"] == instance["active_revision_id"])) ||
      List.first(instance["revisions"]) ||
      %{
        "parameter_schema" => [],
        "parameters" => %{},
        "secret_slot_schema" => [],
        "resource_selection" => %{},
        "runtime_contract" => %{}
      }
  end

  defp parameter_type(declaration) do
    get_in(declaration, ["value_type", "type"]) || declaration["type"] || "string"
  end

  defp parameter_input_type(declaration) do
    case parameter_type(declaration) do
      "integer" -> "number"
      "boolean" -> "checkbox"
      "enum" -> "select"
      _other -> "text"
    end
  end

  defp parameter_options(declaration) do
    get_in(declaration, ["value_type", "values"]) || declaration["values"] || []
  end

  defp parameter_label(declaration, old_value) do
    old =
      cond do
        declaration["sensitive"] -> "current: [REDACTED]"
        is_nil(old_value) or old_value == "" -> "no current value"
        true -> "current: #{old_value}"
      end

    "#{declaration["name"]} · #{old}"
  end

  defp active_parameter(instance, declaration) do
    if declaration["sensitive"] do
      ""
    else
      active_revision(instance)["parameters"][declaration["name"]] || declaration["default"] || ""
    end
  end

  defp active_policy(instance, field) do
    active_revision(instance)["resource_selection"][field] ||
      get_in(active_revision(instance), ["runtime_contract", "policy_ceiling", field]) ||
      if(field == "network", do: "disabled", else: 1)
  end

  defp import_options(imports, slot) do
    imports
    |> Enum.filter(fn secret_import ->
      Enum.any?(secret_import["delivery_modes"], &(&1 in slot["delivery_modes"]))
    end)
    |> Enum.map(fn secret_import ->
      label =
        "#{secret_import["alias"]} · #{secret_import["target_kind"]} · " <>
          Enum.join(secret_import["delivery_modes"], "/")

      {label, secret_import["id"]}
    end)
  end

  defp mode_options(slot) do
    Enum.map(slot["delivery_modes"], fn
      "raw" -> {"Raw file · guest can copy plaintext", "raw"}
      "brokered" -> {"Brokered · value withheld from guest", "brokered"}
    end)
  end

  defp attachment_options(attachments) do
    attachments
    |> Enum.filter(&(&1["enabled"] && is_nil(&1["removed_at"])))
    |> Enum.map(&{"#{&1["repository_name"]} · #{&1["ref_selector"]}", &1["id"]})
  end

  defp human_phase("normal"), do: "Normal runs"
  defp human_phase("update"), do: "Update hooks"

  defp candidate_options(candidates) do
    Enum.map(candidates, &{"#{&1["display_name"]} · #{&1["release_version"]}", &1["id"]})
  end

  defp candidate_schema(instance) do
    case List.first(instance["update_candidates"]) do
      nil -> []
      candidate -> candidate["parameter_schema"]
    end
  end

  defp candidate_default(instance, declaration) do
    if declaration["sensitive"], do: "", else: active_parameter(instance, declaration)
  end

  defp candidate_policy(instance, field) do
    case List.first(instance["update_candidates"]) do
      nil ->
        active_policy(instance, field)

      candidate ->
        get_in(candidate, ["runtime_contract", "policy_ceiling", field]) ||
          active_policy(instance, field)
    end
  end

  defp recovery_actions(%{"state" => state})
       when state in ["compatibility_unknown", "rejected"] do
    [{"Retry hook", "retry"}, {"Reject candidate", "reject"}]
  end

  defp recovery_actions(%{"state" => "activation_recovery"}),
    do: [{"Resume activation", "resume"}]

  defp recovery_actions(_update), do: []

  defp hook_event_summary(%{"event_type" => "vm.log", "payload" => payload}) do
    payload["message"] || "redacted log frame"
  end

  defp hook_event_summary(event), do: inspect(event["payload"], limit: 120, printable_limit: 120)

  defp network_options do
    [
      {"Disabled", "disabled"},
      {"Broker only", "broker_only"},
      {"Constrained egress", "egress"}
    ]
  end

  defp short_id(nil), do: "—"
  defp short_id(value), do: String.slice(value, 0, 8)

  defp attachment_state(%{"removed_at" => removed_at}) when not is_nil(removed_at), do: "removed"
  defp attachment_state(%{"enabled" => true}), do: "enabled"
  defp attachment_state(_attachment), do: "disabled"

  defp attachment_tone(%{"removed_at" => removed_at}) when not is_nil(removed_at), do: "danger"
  defp attachment_tone(%{"enabled" => true}), do: "success"
  defp attachment_tone(_attachment), do: "neutral"

  defp state_tone(value) when value in ["active", "published", "activated"], do: "success"

  defp state_tone(value)
       when value in ["removed", "revoked", "compatibility_unknown", "activation_recovery"],
       do: "danger"

  defp state_tone(_value), do: "neutral"
end
