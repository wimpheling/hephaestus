defmodule HephaestusWeb.RPC.Projection do
  @moduledoc """
  Converts generated response messages into page-owned presentation values.

  Protobuf types stop at the state/effects boundary. This module preserves the
  existing string-keyed presentation contract while keeping the wire contract
  fully typed.
  """

  alias Hephaestus.Artifact.V1.Artifact

  alias Hephaestus.Common.V1.{
    Cursor,
    OpaqueId,
    ParameterDeclaration,
    ParameterDefault,
    ParameterType,
    RuntimeMetric
  }

  alias Hephaestus.Instance.V1.{InstanceRevision, RefSelector, SecretImport, UpdateEvent}
  alias Hephaestus.Release.V1.Release
  alias Hephaestus.Run.V1.{Run, RunEvent}
  alias Hephaestus.Secret.V1.{GrantSummary, ImportSummary, SecretSummary, SecretTarget}

  @enum_prefixes ~w(
    SECRET_SLOT_DELIVERY_MODE_
    SECRET_SLOT_PHASE_
    DIAGNOSTIC_SEVERITY_
    DIAGNOSTIC_CODE_
    OPERATION_STATE_
    NETWORK_POLICY_
    ERROR_CODE_
    DELIVERY_MODE_
    DELIVERY_PHASE_
    SECRET_STATE_
    AUTHORITY_STATE_
    TRIGGER_POLICY_
    RECOVERY_ACTION_
    RECOVERY_DECISION_
    REMOVAL_STATE_
    BUILD_STATE_
    RELEASE_STATE_
    TREE_ENTRY_TYPE_
    RUN_CONTROL_KIND_
    CONTROL_STATE_
    EVENT_SCOPE_KIND_
    AGGREGATE_TYPE_
    CHANGE_KIND_
    LIFECYCLE_STATE_
  )

  @spec to_value(term()) :: term()
  def to_value(nil), do: nil
  def to_value(value) when is_boolean(value), do: value
  def to_value(%Cursor{value: value}), do: blank_to_nil(value)
  def to_value(%OpaqueId{value: value}), do: blank_to_nil(value)

  def to_value(%Google.Protobuf.Timestamp{seconds: seconds, nanos: nanos}) do
    with {:ok, timestamp} <- DateTime.from_unix(seconds, :second) do
      DateTime.add(timestamp, div(nanos, 1_000), :microsecond)
    else
      _invalid -> nil
    end
  end

  def to_value(%ParameterType{constraint: {kind, constraints}}) do
    kind = if kind == :enumeration, do: "enum", else: Atom.to_string(kind)

    constraints
    |> message_map()
    |> Map.put("type", kind)
  end

  def to_value(%ParameterType{constraint: nil}), do: %{"type" => "string"}
  def to_value(%ParameterDefault{value: {_kind, value}}), do: value
  def to_value(%ParameterDefault{value: nil}), do: nil

  def to_value(%ParameterDeclaration{} = declaration) do
    declaration
    |> message_map()
    |> Map.put("type", parameter_kind(declaration.value_type))
    |> Map.put("values", parameter_values(declaration.value_type))
  end

  def to_value(%RuntimeMetric{labels: labels} = metric) do
    metric
    |> message_map()
    |> Map.put("labels", Map.new(labels, &{&1.key, &1.value}))
  end

  def to_value(%SecretTarget{target: {kind, %OpaqueId{} = id}}) do
    %{"target_kind" => target_kind(kind), "target_id" => to_value(id)}
  end

  def to_value(%SecretTarget{target: nil}), do: %{"target_kind" => nil, "target_id" => nil}

  def to_value(%SecretSummary{} = summary) do
    summary
    |> message_map()
    |> rename("state", "status")
  end

  def to_value(%GrantSummary{target: target, policy: policy} = grant) do
    grant
    |> message_map()
    |> Map.drop(["target", "policy"])
    |> Map.merge(to_value(target))
    |> Map.merge(message_map(policy))
    |> rename("state", "status")
    |> rename("import_state", "import_status")
  end

  def to_value(%ImportSummary{target: target, policy: policy} = secret_import) do
    secret_import
    |> message_map()
    |> Map.drop(["target", "policy"])
    |> Map.merge(to_value(target))
    |> Map.merge(message_map(policy))
    |> rename("state", "status")
    |> rename("secret_state", "secret_status")
  end

  def to_value(%SecretImport{target: target, policy: policy} = secret_import) do
    secret_import
    |> message_map()
    |> Map.drop(["target", "policy"])
    |> Map.merge(to_value(target))
    |> Map.merge(message_map(policy))
    |> rename("state", "status")
    |> rename("secret_state", "secret_status")
  end

  def to_value(%RefSelector{selector: {:exact, value}}), do: value
  def to_value(%RefSelector{selector: {:prefix, value}}), do: value <> "/*"
  def to_value(%RefSelector{selector: nil}), do: nil

  def to_value(%InstanceRevision{parameters: parameters} = revision) do
    revision
    |> message_map()
    |> Map.put("parameters", parameter_document(parameters))
  end

  def to_value(%UpdateEvent{payload: payload} = event) do
    event
    |> message_map()
    |> Map.put("payload", event_payload(payload))
  end

  def to_value(%RunEvent{payload: payload} = event) do
    event
    |> message_map()
    |> Map.put("payload", event_payload(payload))
  end

  def to_value(%Artifact{} = artifact) do
    artifact
    |> message_map()
    |> Map.put("content_hash", artifact.sha256)
  end

  def to_value(%Release{build: build} = release) do
    build_map = message_map(build)

    release
    |> message_map()
    |> Map.drop(["build"])
    |> Map.merge(%{
      "build_state" => build_map["state"],
      "build_exit_code" => build_map["exit_code"],
      "build_failure_code" => build_map["failure_code"],
      "build_logs" => build_map["logs"] || [],
      "build_metrics" => build_map["metrics"] || []
    })
  end

  def to_value(%Run{result: result, metrics: metrics} = run) do
    projected = message_map(run)
    result_map = message_map(result)
    proposal_map = result_map |> Map.get("proposal") |> message_map()
    metrics_map = message_map(metrics)

    projected
    |> Map.drop(["result"])
    |> Map.merge(%{
      "result_id" => result_map["id"],
      "result_commit" => result_map["commit"],
      "result_ref" => result_map["ref"],
      "result_tree" => result_map["tree"],
      "result_message" => result_map["message"],
      "artifact_manifest_hash" => result_map["artifact_manifest_hash"],
      "proposal_id" => proposal_map["id"],
      "proposal_state" => proposal_map["state"],
      "target_ref" => proposal_map["target_ref"],
      "proposal_version" => proposal_map["version"],
      "metrics" => Map.drop(metrics_map, ["runtime_metrics"]),
      "runtime_metrics" => metrics_map["runtime_metrics"] || []
    })
  end

  def to_value(list) when is_list(list), do: Enum.map(list, &to_value/1)

  def to_value(value) when is_atom(value) do
    encoded = Atom.to_string(value)

    case Enum.find(@enum_prefixes, &String.starts_with?(encoded, &1)) do
      nil -> encoded
      prefix -> encoded |> String.replace_prefix(prefix, "") |> String.downcase()
    end
  end

  def to_value(%_module{} = message), do: message_map(message)
  def to_value(value), do: value

  defp message_map(nil), do: %{}

  defp message_map(%_module{} = message) do
    message
    |> Map.from_struct()
    |> Map.delete(:__unknown_fields__)
    |> Map.delete(:__protobuf__)
    |> Map.new(fn {key, value} -> {Atom.to_string(key), to_value(value)} end)
  end

  defp message_map(message) when is_map(message) do
    Map.new(message, fn {key, value} -> {to_string(key), to_value(value)} end)
  end

  defp parameter_document(values) do
    Map.new(values, fn value -> {value.name, parameter_value(value.value)} end)
  end

  defp parameter_value({_kind, value}), do: value
  defp parameter_value(nil), do: nil

  defp parameter_kind(%ParameterType{constraint: {kind, _constraints}}) do
    if kind == :enumeration, do: "enum", else: Atom.to_string(kind)
  end

  defp parameter_kind(_type), do: "string"

  defp parameter_values(%ParameterType{constraint: {:enumeration, constraints}}),
    do: constraints.values

  defp parameter_values(_type), do: []

  defp event_payload({:bounded_log_message, message}), do: %{"message" => message}
  defp event_payload({:state, state}), do: %{"state" => state}
  defp event_payload({:metric, metric}), do: to_value(metric)
  defp event_payload({:diagnostic, diagnostic}), do: to_value(diagnostic)
  defp event_payload({:operation_state, state}), do: %{"state" => to_value(state)}
  defp event_payload(nil), do: %{}

  defp target_kind(:project_id), do: "project"
  defp target_kind(:repository_id), do: "repository"

  defp rename(map, old_key, new_key) do
    {value, remainder} = Map.pop(map, old_key)
    Map.put(remainder, new_key, value)
  end

  defp blank_to_nil(""), do: nil
  defp blank_to_nil(value), do: value
end
