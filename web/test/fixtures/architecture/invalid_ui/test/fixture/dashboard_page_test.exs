defmodule Fixture.Invalid.DashboardPageTest do
  use ExUnit.Case

  @covered_states [:ready]

  test "ready state only", do: assert(@covered_states == [:ready])
end
