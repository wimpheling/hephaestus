defmodule Fixture.Invalid.DashboardStateTest do
  use ExUnit.Case

  @covered_statuses [:ready]

  test "omits lifecycle states", do: assert(@covered_statuses == [:ready])
end
