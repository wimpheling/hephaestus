defmodule Mix.Tasks.Hephaestus.Architecture do
  @moduledoc """
  Checks the enabled Phoenix and UI architecture boundaries.

      $ mix hephaestus.architecture
      $ mix hephaestus.architecture --family web
      $ mix hephaestus.architecture --family ui

  Rules are enabled constraint-by-constraint in the Mix project configuration.
  Planned rules remain directly testable with focused fixture trees before the
  corresponding repository migration begins.
  """

  use Mix.Task

  alias Hephaestus.Architecture.Checker
  alias Hephaestus.Architecture.Diagnostic

  @shortdoc "Checks enabled Phoenix and UI architecture rules"

  @impl Mix.Task
  def run(arguments) do
    Mix.Task.run("compile")
    families = parse_families!(arguments)
    enabled_rules = enabled_rules()

    findings =
      Checker.check(File.cwd!(), families: families, enabled_rules: enabled_rules)

    if findings == [] do
      describe_success(families, enabled_rules)
    else
      Enum.each(findings, &Mix.shell().error(Diagnostic.format(&1)))
      Mix.raise("architecture checks failed with #{length(findings)} violation(s)")
    end
  end

  defp parse_families!(arguments) do
    {options, positional, invalid} =
      OptionParser.parse(arguments, strict: [family: :string], aliases: [f: :family])

    if positional != [] or invalid != [] do
      Mix.raise("usage: mix hephaestus.architecture [--family web|ui]")
    end

    case Keyword.get_values(options, :family) do
      [] -> [:web, :ui]
      ["web"] -> [:web]
      ["ui"] -> [:ui]
      _other -> Mix.raise("--family must be exactly one of: web, ui")
    end
  end

  defp enabled_rules do
    Mix.Project.config()
    |> Keyword.get(:hephaestus_architecture, [])
    |> Keyword.get(:enabled_rules, [])
  end

  defp describe_success(families, enabled_rules) do
    selected = Enum.flat_map(families, &Checker.rules/1)
    active = Enum.filter(selected, &(&1 in enabled_rules))

    if active == [] do
      Mix.shell().info(
        "hephaestus architecture: no #{family_names(families)} rules are enabled for the current migration constraint"
      )
    else
      Mix.shell().info("hephaestus architecture: #{length(active)} enabled rule(s) passed")
    end
  end

  defp family_names([:web, :ui]), do: "WEB/UI"
  defp family_names([family]), do: family |> Atom.to_string() |> String.upcase()
end
