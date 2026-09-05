defmodule HephaestusWebWeb.ProjectGatewaysStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProjectGatewaysState

  @covered_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  test "covers the gateway collection lifecycle contract" do
    assert @covered_statuses == ProjectGatewaysState.statuses()
    assert ProjectGatewaysState.stream_mode() == :none
  end
end
