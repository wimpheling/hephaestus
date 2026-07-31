defmodule Fixture.BackendClient do
  def fetch, do: Req.get("http://application.internal/resources")
end
