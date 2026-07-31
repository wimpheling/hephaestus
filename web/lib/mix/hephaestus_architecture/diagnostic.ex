defmodule Hephaestus.Architecture.Diagnostic do
  @moduledoc """
  A stable, actionable architecture-checker finding.
  """

  @enforce_keys [:rule, :path, :line, :message]
  defstruct [:rule, :path, :line, :message]

  @type t :: %__MODULE__{
          rule: String.t(),
          path: String.t(),
          line: pos_integer(),
          message: String.t()
        }

  @doc "Returns the stable human-readable representation of a finding."
  @spec format(t()) :: String.t()
  def format(%__MODULE__{} = diagnostic) do
    "#{diagnostic.path}:#{diagnostic.line}: [#{diagnostic.rule}] #{diagnostic.message} " <>
      "(ARCHITECTURE.md#architecture-rule-index)"
  end
end
