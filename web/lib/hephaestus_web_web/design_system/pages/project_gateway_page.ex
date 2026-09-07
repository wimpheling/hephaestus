defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectGatewayPage do
  @moduledoc "Pure presentation for an authorized gateway and its ingress audit summary."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :gateway, :map, default: nil
  attr :ingress, :list, default: []
  attr :gateways_destination, :string, required: true
  attr :lifecycle_event, :string, required: true, values: ["lifecycle"]
  attr :lifecycle_actions, :list, default: []
  attr :release_catalog, :list, default: []
  attr :secret_authority, :map, default: %{"imports" => []}
  attr :secrets, :list, default: []
  attr :instances, :list, default: []
  attr :bindings, :list, default: []
  attr :binding_slots, :list, default: []
  attr :binding_form, :map, default: %{}
  attr :configure_form, :map, default: %{}
  attr :configure_event, :string, default: "configure", values: ["configure"]
  attr :bind_event, :string, default: "bind-mailbox", values: ["bind-mailbox"]

  @doc "Renders a gateway detail view from its presentation model."
  def project_gateway_page(assigns) do
    ~H"""
    <.page_state
      id="project-gateway-page-state"
      state={@state}
      title="Gateway unavailable"
      message="Gateway information is not ready."
    >
      <.frame id="project-gateway" variant={:summary_body}>
        <.breadcrumbs id="project-gateway-breadcrumbs">
          <:item navigate={@gateways_destination}>Gateways</:item>
          <:current>{@gateway["name"]}</:current>
        </.breadcrumbs>
        <.page_heading
          eyebrow="Project ingress"
          title={@gateway["name"]}
          description="Immutable gateway revisions and authorized ingress outcomes."
        >
          <:actions>
            <.tag>Lifecycle: {@gateway["lifecycle"]}</.tag>
            <.action
              :for={action <- @lifecycle_actions}
              interaction={:event}
              event={@lifecycle_event}
              event_payload={%{next: action.next}}
              variant={action.variant}
            >
              {action.label}
            </.action>
          </:actions>
        </.page_heading>
        <.frame as="section" id="gateway-revisions" variant={:table}>
          <.page_heading eyebrow="Immutable revisions" title="Routes" level="h2" />
          <.resource_list id="gateway-revision-list" layout={:projects}>
            <:header>
              <.text as="span" variant={:muted}>Handler contract</.text>
              <.text as="span" variant={:muted}>Routes</.text>
            </:header>
            <:empty>No revisions are available.</:empty>
            <.frame
              :for={revision <- @gateway["revisions"] || []}
              as="article"
              id={"gateway-revision-#{revision["id"] || revision["handler_contract"]}"}
              variant={:table_row}
            >
              <.text as="strong">{revision["handler_contract"]}</.text>
              <.frame variant={:resource_detail}>
                <.text as="small" variant={:muted}>{revision["exposure"]}</.text>
                <.text :for={route <- revision["routes"] || []} as="small">
                  {route["path"]} · {Enum.join(route["methods"] || [], ", ")}
                </.text>
              </.frame>
            </.frame>
          </.resource_list>
        </.frame>
        <.frame as="section" id="gateway-configuration" variant={:panel}>
          <.page_heading
            eyebrow="Runtime configuration"
            title="Configure a new revision"
            description="Published parameters and explicit secret selections create an immutable revision. Existing mailbox bindings are not copied."
            level="h2"
          />
          <.form_container
            for={to_form(@configure_form, as: :configure)}
            id="configure-gateway-form"
            submit={@configure_event}
          >
            <.input
              :for={parameter <- parameter_schema(@gateway, @release_catalog)}
              id={"configure-parameter-#{parameter["name"]}"}
              name={"configure[parameters][#{parameter["name"]}]"}
              value={parameter_default(parameter["default"])}
              type={parameter_type(parameter)}
              options={parameter_options(parameter)}
              label={parameter["name"]}
              required={parameter["required"]}
            />
            <.input
              :for={slot <- active_secret_slots(@gateway)}
              id={"configure-secret-#{slot}"}
              name={"configure[secret_selections][#{slot}]"}
              value={get_in(@configure_form, ["secret_selections", slot]) || ""}
              type="select"
              label={"Secret for #{slot} (optional)"}
              options={secret_options(@secret_authority, @secrets, active_routes(@gateway))}
            />
            <.text as="small" variant={:muted}>
              New revisions require explicit mailbox bindings before publication is enabled.
            </.text>
            <.action interaction={:submit} variant={:primary}>Create immutable revision</.action>
          </.form_container>
        </.frame>
        <.frame as="section" id="gateway-mailbox-bindings" variant={:panel}>
          <.page_heading
            eyebrow="Mailbox publication"
            title="Bind an authorized mailbox"
            description="Select a mailbox you can manage and a slot declared by the active immutable revision. Bindings are explicit and are never copied during configuration."
            level="h2"
          />
          <.form_container
            for={to_form(@binding_form, as: :gateway_binding)}
            id="gateway-binding-form"
            submit={@bind_event}
          >
            <.input
              id="gateway-binding-slot"
              name="gateway_binding[slot_key]"
              value={@binding_form["slot_key"] || ""}
              type="select"
              label="Declared mailbox slot"
              options={Enum.map(@binding_slots, &{&1, &1})}
              prompt="Choose a declared slot"
              required
            />
            <.input
              id="gateway-binding-mailbox"
              name="gateway_binding[mailbox_id]"
              value={@binding_form["mailbox_id"] || ""}
              type="select"
              label="Authorized mailbox"
              options={mailbox_options(@instances)}
              prompt="Choose an instance mailbox"
              required
            />
            <.input
              id="gateway-binding-producer"
              name="gateway_binding[producer_id]"
              value={@binding_form["producer_id"] || "project-gateway"}
              label="Producer identity"
              required
            />
            <.text :if={@binding_slots == []} as="small" variant={:muted}>
              The active revision declares no mailbox slots.
            </.text>
            <.text
              :if={@binding_slots != [] and mailbox_options(@instances) == []}
              as="small"
              variant={:muted}
            >
              No authorized instance mailbox is available. Allocate one from its instance page first.
            </.text>
            <.action
              :if={@binding_slots != [] and mailbox_options(@instances) != []}
              interaction={:submit}
              variant={:primary}
            >
              Create mailbox binding
            </.action>
          </.form_container>
          <.resource_list id="gateway-binding-list" layout={:projects}>
            <:header>
              <.text as="span" variant={:muted}>Slot</.text>
              <.text as="span" variant={:muted}>Mailbox</.text>
              <.text as="span" variant={:muted}>Grant</.text>
            </:header>
            <:empty>No mailbox bindings exist for the active revision.</:empty>
            <.frame
              :for={binding <- @bindings}
              as="article"
              id={"gateway-binding-#{binding["id"]}"}
              variant={:table_row}
            >
              <.text as="strong">{binding["slot_key"]}</.text>
              <.text as="small">{binding["mailbox_id"]}</.text>
              <.tag>{binding["grant_status"]}</.tag>
            </.frame>
          </.resource_list>
        </.frame>
        <.frame as="section" id="gateway-ingress" variant={:table}>
          <.page_heading eyebrow="Ingress audit" title="Recent outcomes" level="h2" />
          <.resource_list id="gateway-ingress-list" layout={:projects}>
            <:header>
              <.text as="span" variant={:muted}>Outcome</.text>
              <.text as="span" variant={:muted}>Accepted at</.text>
            </:header>
            <:empty>No ingress records are available.</:empty>
            <.frame
              :for={entry <- @ingress}
              as="article"
              id={"gateway-ingress-#{entry["id"] || entry["accepted_at"]}"}
              variant={:table_row}
            >
              <.tag>{entry["outcome"]}</.tag>
              <.text as="time">{entry["accepted_at"]}</.text>
            </.frame>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp parameter_schema(gateway, catalog) do
    release_agent_id = get_in(active_revision(gateway), ["release_agent_id"])

    catalog
    |> Enum.find(&(&1["id"] == release_agent_id))
    |> case do
      nil -> []
      agent -> agent["parameter_schema"] || []
    end
  end

  defp active_revision(%{"active_revision_id" => active, "revisions" => revisions}),
    do: Enum.find(revisions, &(&1["id"] == active)) || %{}

  defp active_revision(_gateway), do: %{}

  defp active_secret_slots(gateway), do: active_revision(gateway)["secret_slots"] || []

  defp active_routes(gateway), do: active_revision(gateway)["routes"] || []

  defp mailbox_options(instances) do
    instances
    |> Enum.filter(fn instance ->
      instance["state"] not in ["removed", "REMOVED"] and is_binary(instance["mailbox_id"]) and
        instance["mailbox_id"] != ""
    end)
    |> Enum.map(fn instance ->
      {"#{instance["name"] || "Instance"} (#{instance["mailbox_id"]})", instance["mailbox_id"]}
    end)
  end

  defp parameter_type(%{"value_type" => %{"type" => "integer"}}), do: "number"
  defp parameter_type(%{"value_type" => %{"type" => "boolean"}}), do: "select"
  defp parameter_type(_parameter), do: "text"

  defp parameter_options(%{"value_type" => %{"type" => "boolean"}}),
    do: [{"True", "true"}, {"False", "false"}]

  defp parameter_options(%{"value_type" => %{"type" => "enum", "values" => values}}),
    do: Enum.map(values, &{&1, &1})

  defp parameter_options(_parameter), do: nil

  defp parameter_default(nil), do: ""
  defp parameter_default(value), do: to_string(value)

  defp secret_options(authority, secrets, routes) do
    route = List.first(routes) || %{}
    route_path = route["path"] || ""
    header_name = secret_header(route_path)

    options =
      authority
      |> Map.get("imports", [])
      |> Enum.filter(&active_import?/1)
      |> Enum.flat_map(fn import ->
        secret = Enum.find(secrets, &(&1["id"] == import["secret_id"]))
        version = import["active_version_id"] || (secret && secret["active_version_id"])

        if is_binary(version) and version != "" and active_secret?(secret) do
          [
            {"#{import["alias"] || import["secret_name"]} (current version)",
             "#{import["id"]}|#{version}|#{route_path}|#{header_name}"}
          ]
        else
          []
        end
      end)

    [{"No secret selected", ""} | options]
  end

  defp active_import?(import),
    do: (Map.get(import, "status") || Map.get(import, "state")) == "active"

  defp active_secret?(nil), do: true
  defp active_secret?(secret), do: Map.get(secret, "status", "active") == "active"

  defp secret_header("/cooking/telegram"), do: "x-telegram-bot-api-secret-token"
  defp secret_header(_route_path), do: "x-heph-secret"
end
