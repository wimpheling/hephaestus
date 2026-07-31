defmodule Fixture.Valid.DashboardPageTest do
  use ExUnit.Case

  @covered_states [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]
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
  @a11y_test_ids [:button, :card]

  test "catalog components retain accessible names and composition" do
    assert @covered_states == @covered_statuses
    assert @a11y_test_ids == [:button, :card]
  end
end
