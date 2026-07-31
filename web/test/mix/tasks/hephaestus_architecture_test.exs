defmodule Mix.Tasks.Hephaestus.ArchitectureTest do
  use ExUnit.Case, async: true

  import ExUnit.CaptureIO

  alias Hephaestus.Architecture.Checker
  alias Hephaestus.Architecture.Diagnostic

  @fixtures Path.expand("../../fixtures/architecture", __DIR__)

  test "valid dependency, AST placement, and HEEx fixtures pass every implemented rule" do
    assert Checker.check(fixture("valid"), enabled_rules: :all) == []
  end

  test "repository rules stay disabled until their migration constraint begins" do
    assert Checker.check(fixture("invalid_web"), enabled_rules: []) == []
    assert Checker.check(fixture("invalid_ui"), enabled_rules: []) == []
  end

  test "WEB dependency inspection reports forbidden Mix dependencies" do
    findings =
      Checker.check(fixture("invalid_web"),
        families: [:web],
        enabled_rules: ["WEB-NO-INFRASTRUCTURE-DEPENDENCIES"]
      )

    messages = Enum.map(findings, & &1.message)
    assert Enum.any?(messages, &String.contains?(&1, ":ecto_sql"))
    assert Enum.any?(messages, &String.contains?(&1, ":postgrex"))
    assert Enum.any?(messages, &String.contains?(&1, ":icons from Git"))
    assert Enum.any?(messages, &String.contains?(&1, "infrastructure module Ecto.Query"))
    assert Enum.any?(messages, &String.contains?(&1, "infrastructure module Postgrex"))
    assert Enum.any?(messages, &String.contains?(&1, "infrastructure module Gnat"))
    assert Enum.any?(messages, &String.contains?(&1, "environment variable DATABASE_URL"))
    assert Enum.any?(messages, &String.contains?(&1, "SQL literal SELECT"))
  end

  test "WEB client inspection rejects hand-written application HTTP" do
    findings =
      Checker.check(fixture("invalid_web"),
        families: [:web],
        enabled_rules: ["WEB-NO-HANDWRITTEN-BACKEND-CLIENT"]
      )

    assert Enum.any?(findings, &String.contains?(&1.message, "Req.get"))
  end

  test "WEB error inspection rejects unrestricted backend text" do
    findings =
      Checker.check(fixture("invalid_web"),
        families: [:web],
        enabled_rules: ["WEB-NO-RAW-BACKEND-ERROR"]
      )

    assert Enum.any?(findings, &String.contains?(&1.message, "inspect/1"))
  end

  test "WEB host-I/O inspection rejects filesystem and subprocess calls" do
    findings =
      Checker.check(fixture("invalid_web"),
        families: [:web],
        enabled_rules: ["WEB-NO-FILESYSTEM-OR-PROCESS"]
      )

    messages = Enum.map(findings, & &1.message)
    assert Enum.any?(messages, &String.contains?(&1, "File.read!"))
    assert Enum.any?(messages, &String.contains?(&1, "System.cmd"))
  end

  test "WEB AST inspection permits state calls and rejects a generated client call in LiveView" do
    findings =
      Checker.check(fixture("invalid_web"),
        families: [:web],
        enabled_rules: ["WEB-RPC-CLIENTS-ONLY-IN-STATE"]
      )

    assert [%{path: "lib/fixture/live/dashboard_live.ex"} = finding] = findings
    assert finding.message =~ "DashboardClient.list_dashboards"
  end

  test "HEEx parser rejects raw controls, layout, and SVG but ignores comments and components" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-RAW-HTML-ONLY-IN-COMPONENTS"]
      )

    assert Enum.map(findings, & &1.message) == [
             "raw <main> tag is outside the basic design-system component tier; render a public component instead",
             "raw <input> tag is outside the basic design-system component tier; render a public component instead",
             "raw <button> tag is outside the basic design-system component tier; render a public component instead",
             "raw <svg> tag is outside the basic design-system component tier; render a public component instead"
           ]
  end

  test "tier inspection rejects upward dependencies and misplaced modules" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-TIER-DIRECTION"]
      )

    assert Enum.any?(findings, &String.contains?(&1.message, "depends upward"))
    assert Enum.any?(findings, &String.contains?(&1.message, "imports implementation module"))
    assert Enum.any?(findings, &String.contains?(&1.message, "not the Components namespace"))
  end

  test "bounded styling rule rejects class, error, global, style, layout, and page escapes" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-NO-CLASS-ESCAPE-HATCH"]
      )

    messages = Enum.map(findings, & &1.message)
    assert Enum.any?(messages, &String.contains?(&1, "property :class"))
    assert Enum.any?(messages, &String.contains?(&1, "property :error_class"))
    assert Enum.any?(messages, &String.contains?(&1, "global property :rest"))
    assert Enum.any?(messages, &String.contains?(&1, "property :style"))
    assert Enum.any?(messages, &String.contains?(&1, "layout property :columns"))
    assert Enum.any?(messages, &String.contains?(&1, "class is authored outside"))
  end

  test "declared interaction rule rejects literal event names and options" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-DECLARED-INTERACTIONS-ONLY"]
      )

    messages = Enum.map(findings, & &1.message)
    assert Enum.any?(messages, &String.contains?(&1, "interaction property :on_retry"))
    assert Enum.any?(messages, &String.contains?(&1, "phx-click"))
    assert Enum.any?(messages, &String.contains?(&1, "phx-value-kind"))
  end

  test "design token rule rejects page CSS literals and HEEx styling" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-DESIGN-TOKENS-ONLY"]
      )

    messages = Enum.map(findings, & &1.message)
    assert Enum.any?(messages, &String.contains?(&1, "literal color"))
    assert Enum.any?(messages, &String.contains?(&1, "literal font"))
    assert Enum.any?(messages, &String.contains?(&1, "literal radius"))
    assert Enum.any?(messages, &String.contains?(&1, "literal shadow"))
    assert Enum.any?(messages, &String.contains?(&1, "unapproved spacing"))
    assert Enum.any?(messages, &String.contains?(&1, "bypasses centralized design tokens"))
  end

  test "external UI imports are rejected outside basic component wrappers" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-NO-EXTERNAL-UI-IMPORTS"]
      )

    assert Enum.any?(findings, &String.contains?(&1.message, "Heroicons.Outline"))
  end

  test "DOM injection scanner rejects equivalent APIs but ignores strings and designated hooks" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-NO-DOM-INJECTION"]
      )

    messages = Enum.map(findings, & &1.message)
    assert Enum.any?(messages, &String.contains?(&1, "innerHTML"))
    assert Enum.any?(messages, &String.contains?(&1, "outerHTML"))
    assert Enum.any?(messages, &String.contains?(&1, "insertAdjacentHTML"))
    assert Enum.any?(messages, &String.contains?(&1, "raw DOM creation"))
    assert Enum.any?(messages, &String.contains?(&1, "DOMParser"))
    assert Enum.any?(messages, &String.contains?(&1, "markup parsing"))
    assert Enum.any?(messages, &String.contains?(&1, "contextual fragment"))
    assert Enum.any?(messages, &String.contains?(&1, "document.write"))
  end

  test "public facade catalog rejects undeclared implementation exports" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-PUBLIC-FACADE-COMPLETE"]
      )

    assert Enum.any?(findings, &String.contains?(&1.message, "Panel.panel/1 has no facade"))
  end

  test "showcase and accessibility manifests must exactly cover the facade catalog" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-SHOWCASE-AND-TEST-PARITY"]
      )

    messages = Enum.map(findings, & &1.message)
    assert Enum.any?(messages, &String.contains?(&1, "showcase module is missing"))
    assert Enum.any?(messages, &String.contains?(&1, "missing showcase parity entry"))
    assert Enum.any?(messages, &String.contains?(&1, "missing accessibility test parity entry"))
  end

  test "page state coverage must match every declared render state" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: ["UI-PAGE-STATE-COVERAGE"]
      )

    assert Enum.any?(findings, &String.contains?(&1.message, "does not match declared states"))
  end

  test "diagnostics are stable, actionable, and link the normative rule" do
    finding =
      fixture("invalid_web")
      |> Checker.check(
        families: [:web],
        enabled_rules: ["WEB-NO-INFRASTRUCTURE-DEPENDENCIES"]
      )
      |> Enum.find(&(&1.path == "mix.exs"))

    formatted = Diagnostic.format(finding)

    assert formatted =~ "mix.exs:1: [WEB-NO-INFRASTRUCTURE-DEPENDENCIES]"
    assert formatted =~ "ARCHITECTURE.md#architecture-rule-index"
  end

  test "LiveView structure checks require companions, one page render, pure state, and pure pages" do
    findings =
      Checker.check(fixture("invalid_ui"),
        families: [:ui],
        enabled_rules: [
          "UI-PAGE-COMPANIONS",
          "UI-LIVE-RENDERS-ONE-PAGE",
          "UI-STATE-HAS-NO-HEEX",
          "UI-PAGE-IS-PURE"
        ]
      )

    rules = MapSet.new(findings, & &1.rule)
    assert MapSet.member?(rules, "UI-PAGE-COMPANIONS")
    assert MapSet.member?(rules, "UI-LIVE-RENDERS-ONE-PAGE")
    assert MapSet.member?(rules, "UI-STATE-HAS-NO-HEEX")
    assert MapSet.member?(rules, "UI-PAGE-IS-PURE")

    messages = Enum.map(findings, & &1.message)

    assert Enum.any?(messages, &String.contains?(&1, "missing its state test companion"))
    assert Enum.any?(messages, &String.contains?(&1, "state test must declare exact"))
    assert Enum.any?(messages, &String.contains?(&1, "page render test must cover all eight"))
    assert Enum.any?(messages, &String.contains?(&1, "exactly one page component"))
    assert Enum.any?(messages, &String.contains?(&1, "non-callback backend/0"))
    assert Enum.any?(messages, &String.contains?(&1, "socket key :page_state"))
    assert Enum.any?(messages, &String.contains?(&1, "sensitive field :secret_token"))
    assert Enum.any?(messages, &String.contains?(&1, "sensitive field :password"))
    assert Enum.any?(messages, &String.contains?(&1, "product client/service"))
    assert Enum.any?(messages, &String.contains?(&1, "page-local runtime"))
    assert Enum.any?(messages, &String.contains?(&1, "page-scoped stream"))
    assert Enum.any?(messages, &String.contains?(&1, "HEEx rendering"))
    assert Enum.any?(messages, &String.contains?(&1, "rendering/runtime reference"))
    assert Enum.any?(messages, &String.contains?(&1, "exact literal @statuses"))
    assert Enum.any?(messages, &String.contains?(&1, "defstruct must use exactly"))
    assert Enum.any?(messages, &String.contains?(&1, "must define new/1"))
    assert Enum.any?(messages, &String.contains?(&1, "LiveView callback handle_event"))
    assert Enum.any?(messages, &String.contains?(&1, "runtime dependency Fixture.Generated"))

    assert Enum.any?(
             messages,
             &String.contains?(&1, "runtime dependency Fixture.DashboardService")
           )

    assert Enum.any?(messages, &String.contains?(&1, "socket access"))
  end

  test "Mix task runs every enabled WEB rule" do
    Mix.Task.reenable("hephaestus.architecture")

    output = capture_io(fn -> Mix.Tasks.Hephaestus.Architecture.run(["--family", "web"]) end)

    assert output =~ "5 enabled rule(s) passed"
  end

  test "Mix task rejects an unknown family" do
    Mix.Task.reenable("hephaestus.architecture")

    assert_raise Mix.Error, ~r/--family must be exactly one of/, fn ->
      Mix.Tasks.Hephaestus.Architecture.run(["--family", "database"])
    end
  end

  defp fixture(name), do: Path.join(@fixtures, name)
end
