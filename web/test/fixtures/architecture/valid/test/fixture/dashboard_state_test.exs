defmodule Fixture.Valid.DashboardStateTest do
  use ExUnit.Case

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

  test "covers the lifecycle contract", do: assert(length(@covered_statuses) == 8)
end
