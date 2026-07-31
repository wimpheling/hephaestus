defmodule Fixture.HostIo do
  def read, do: File.read!("/srv/repositories/private")
  def git, do: System.cmd("git", ["status"])
end
