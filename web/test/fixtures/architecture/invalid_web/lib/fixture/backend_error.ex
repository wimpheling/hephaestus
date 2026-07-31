defmodule Fixture.BackendError do
  def present(reason), do: "Backend failed: #{inspect(reason)}"
end
