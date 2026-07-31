defmodule HephaestusWeb.RPC.ProjectionTest do
  use ExUnit.Case, async: true

  alias Hephaestus.Common.V1.{
    EnumParameterConstraints,
    MetricLabel,
    OpaqueId,
    ParameterDeclaration,
    ParameterDefault,
    ParameterType,
    RuntimeMetric
  }

  alias Hephaestus.Secret.V1.SecretSummary
  alias Hephaestus.Run.V1.{ResultProposal, Run, RunMetrics, RunResult}
  alias HephaestusWeb.RPC.Projection

  test "projects parameter schema constraints, defaults, and sensitivity" do
    declaration = %ParameterDeclaration{
      name: "tier",
      label: "Tier",
      value_type: %ParameterType{
        constraint: {:enumeration, %EnumParameterConstraints{values: ["small", "large"]}}
      },
      required: true,
      default: %ParameterDefault{value: {:string_value, "small"}},
      sensitive: true
    }

    assert %{
             "name" => "tier",
             "type" => "enum",
             "values" => ["small", "large"],
             "default" => "small",
             "sensitive" => true
           } = Projection.to_value(declaration)
  end

  test "projects enum states and opaque identifiers into the page contract" do
    secret = %SecretSummary{
      id: %OpaqueId{value: "018f53e7-4dda-78e6-b1b6-a7bcba528e0d"},
      name: "deploy-token",
      state: :SECRET_STATE_ACTIVE,
      allowed_delivery_modes: [:DELIVERY_MODE_BROKERED]
    }

    assert %{
             "id" => "018f53e7-4dda-78e6-b1b6-a7bcba528e0d",
             "status" => "active",
             "allowed_delivery_modes" => ["brokered"]
           } = Projection.to_value(secret)
  end

  test "projects runtime metric labels as a bounded string map" do
    metric = %RuntimeMetric{
      name: "duration",
      value: 1.25,
      unit: "seconds",
      labels: [%MetricLabel{key: "phase", value: "update"}]
    }

    assert %{"labels" => %{"phase" => "update"}} = Projection.to_value(metric)
  end

  test "projects a run result proposal after nested messages are normalized" do
    run = %Run{
      result: %RunResult{
        id: %OpaqueId{value: "result-id"},
        proposal: %ResultProposal{
          id: %OpaqueId{value: "proposal-id"},
          state: "open",
          target_ref: "refs/heads/main",
          version: 2
        }
      },
      metrics: %RunMetrics{}
    }

    assert %{
             "result_id" => "result-id",
             "proposal_id" => "proposal-id",
             "proposal_state" => "open",
             "target_ref" => "refs/heads/main",
             "proposal_version" => 2
           } = Projection.to_value(run)
  end
end
