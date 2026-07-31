defmodule Hephaestus.Architecture.Checker do
  @moduledoc """
  Stable structural checks for the Phoenix and design-system boundaries.

  Repository-wide rules are selected explicitly so a later migration's rules
  can be fixture-tested before they become a hard gate.
  """

  alias Hephaestus.Architecture.Diagnostic

  @web_dependency_rule "WEB-NO-INFRASTRUCTURE-DEPENDENCIES"
  @web_rpc_rule "WEB-RPC-CLIENTS-ONLY-IN-STATE"
  @web_client_rule "WEB-NO-HANDWRITTEN-BACKEND-CLIENT"
  @web_error_rule "WEB-NO-RAW-BACKEND-ERROR"
  @web_io_rule "WEB-NO-FILESYSTEM-OR-PROCESS"
  @ui_raw_html_rule "UI-RAW-HTML-ONLY-IN-COMPONENTS"
  @ui_tier_rule "UI-TIER-DIRECTION"
  @ui_companions_rule "UI-PAGE-COMPANIONS"
  @ui_one_page_rule "UI-LIVE-RENDERS-ONE-PAGE"
  @ui_state_rule "UI-STATE-HAS-NO-HEEX"
  @ui_pure_page_rule "UI-PAGE-IS-PURE"
  @ui_interaction_rule "UI-DECLARED-INTERACTIONS-ONLY"
  @ui_class_rule "UI-NO-CLASS-ESCAPE-HATCH"
  @ui_token_rule "UI-DESIGN-TOKENS-ONLY"
  @ui_external_rule "UI-NO-EXTERNAL-UI-IMPORTS"
  @ui_dom_injection_rule "UI-NO-DOM-INJECTION"
  @ui_facade_rule "UI-PUBLIC-FACADE-COMPLETE"
  @ui_parity_rule "UI-SHOWCASE-AND-TEST-PARITY"
  @ui_page_state_rule "UI-PAGE-STATE-COVERAGE"

  @page_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]
  @page_state_field_names [:status, :data, :form, :error, :cursor, :stream_generation]
  @live_callbacks [
    mount: 3,
    handle_params: 3,
    handle_event: 3,
    handle_info: 2,
    handle_async: 3,
    terminate: 2,
    render: 1
  ]

  @rules [
    @web_dependency_rule,
    @web_rpc_rule,
    @web_client_rule,
    @web_error_rule,
    @web_io_rule,
    @ui_raw_html_rule,
    @ui_tier_rule,
    @ui_companions_rule,
    @ui_one_page_rule,
    @ui_state_rule,
    @ui_pure_page_rule,
    @ui_interaction_rule,
    @ui_class_rule,
    @ui_token_rule,
    @ui_external_rule,
    @ui_dom_injection_rule,
    @ui_facade_rule,
    @ui_parity_rule,
    @ui_page_state_rule
  ]
  @web_rules [
    @web_dependency_rule,
    @web_rpc_rule,
    @web_client_rule,
    @web_error_rule,
    @web_io_rule
  ]

  @ui_rules [
    @ui_raw_html_rule,
    @ui_tier_rule,
    @ui_companions_rule,
    @ui_one_page_rule,
    @ui_state_rule,
    @ui_pure_page_rule,
    @ui_interaction_rule,
    @ui_class_rule,
    @ui_token_rule,
    @ui_external_rule,
    @ui_dom_injection_rule,
    @ui_facade_rule,
    @ui_parity_rule,
    @ui_page_state_rule
  ]

  @forbidden_dependencies [
    :ecto_sql,
    :phoenix_ecto,
    :postgrex,
    :nats,
    :gnat,
    :broadway_nats
  ]

  @forbidden_infrastructure_modules [
    "Ecto",
    "Postgrex",
    "DBConnection",
    "Gnat",
    "Nats",
    "BroadwayNats",
    "HephaestusWeb.Repo",
    "HephaestusWeb.Store",
    "HephaestusWeb.RunNotifier",
    "HephaestusWeb.RepositoryBrowser",
    "HephaestusWeb.ArtifactStore"
  ]

  @forbidden_backend_clients ["Req", "Finch", "HTTPoison", "Tesla", "Mint.HTTP"]
  @forbidden_infrastructure_env ~w(
    DATABASE_URL
    HEPHAESTUS_DATABASE_URL
    NATS_URL
    HEPHAESTUS_NATS_URL
    HEPHAESTUS_REPOSITORY_ROOT
    HEPHAESTUS_ARTIFACT_ROOT
  )

  @type family :: :web | :ui

  @doc "Returns all rule IDs implemented by this checker."
  @spec rules() :: [String.t()]
  def rules, do: @rules

  @doc "Returns the rule IDs belonging to a checker family."
  @spec rules(family()) :: [String.t()]
  def rules(:web), do: @web_rules
  def rules(:ui), do: @ui_rules

  @doc "Checks `root` using only the explicitly enabled and selected rules."
  @spec check(Path.t(), keyword()) :: [Diagnostic.t()]
  def check(root, options \\ []) do
    root = Path.expand(root)
    families = Keyword.get(options, :families, [:web, :ui])
    enabled_rules = Keyword.get(options, :enabled_rules, [])

    selected_rules =
      families
      |> Enum.flat_map(&rules/1)
      |> Enum.filter(&enabled?(&1, enabled_rules))

    selected_rules
    |> Enum.flat_map(&check_rule(root, &1))
    |> Enum.sort_by(&{&1.path, &1.line, &1.rule, &1.message})
  end

  defp enabled?(_rule, :all), do: true
  defp enabled?(rule, enabled_rules), do: rule in enabled_rules

  defp check_rule(root, @web_dependency_rule), do: check_dependencies(root)
  defp check_rule(root, @web_rpc_rule), do: check_rpc_calls(root)
  defp check_rule(root, @web_client_rule), do: check_backend_clients(root)
  defp check_rule(root, @web_error_rule), do: check_backend_errors(root)
  defp check_rule(root, @web_io_rule), do: check_filesystem_and_process(root)
  defp check_rule(root, @ui_raw_html_rule), do: check_raw_html(root)
  defp check_rule(root, @ui_tier_rule), do: check_tier_direction(root)
  defp check_rule(root, @ui_companions_rule), do: check_page_companions(root)
  defp check_rule(root, @ui_one_page_rule), do: check_live_renders(root)
  defp check_rule(root, @ui_state_rule), do: check_state_modules(root)
  defp check_rule(root, @ui_pure_page_rule), do: check_pure_pages(root)
  defp check_rule(root, @ui_interaction_rule), do: check_declared_interactions(root)
  defp check_rule(root, @ui_class_rule), do: check_class_escape_hatches(root)
  defp check_rule(root, @ui_token_rule), do: check_design_tokens(root)
  defp check_rule(root, @ui_external_rule), do: check_external_ui_imports(root)
  defp check_rule(root, @ui_dom_injection_rule), do: check_dom_injection(root)
  defp check_rule(root, @ui_facade_rule), do: check_public_facade(root)
  defp check_rule(root, @ui_parity_rule), do: check_showcase_and_test_parity(root)
  defp check_rule(root, @ui_page_state_rule), do: check_page_state_coverage(root)

  defp check_dependencies(root) do
    path = Path.join(root, "mix.exs")

    with {:ok, source} <- File.read(path),
         {:ok, ast} <- Code.string_to_quoted(source, file: path) do
      {_ast, dependencies} =
        Macro.prewalk(ast, MapSet.new(), fn
          {:{}, _meta, [dependency | options]} = node, dependencies
          when is_atom(dependency) ->
            cond do
              dependency in @forbidden_dependencies ->
                {node, MapSet.put(dependencies, dependency)}

              git_dependency_options?(options) ->
                {node, MapSet.put(dependencies, {:git_source, dependency})}

              true ->
                {node, dependencies}
            end

          {dependency, _requirement} = node, dependencies
          when dependency in @forbidden_dependencies ->
            {node, MapSet.put(dependencies, dependency)}

          {dependency, options} = node, dependencies
          when is_atom(dependency) and is_list(options) ->
            if git_dependency_options?([options]) do
              {node, MapSet.put(dependencies, {:git_source, dependency})}
            else
              {node, dependencies}
            end

          node, dependencies ->
            {node, dependencies}
        end)

      dependency_findings =
        dependencies
        |> Enum.sort()
        |> Enum.map(fn dependency ->
          dependency_label =
            case dependency do
              {:git_source, name} -> ":#{name} from Git"
              name -> ":#{name}"
            end

          diagnostic(
            root,
            path,
            1,
            @web_dependency_rule,
            "Phoenix declares forbidden infrastructure/backend dependency #{dependency_label}; " <>
              "remove it and use generated RPC stubs from the state layer"
          )
        end)

      dependency_findings ++ infrastructure_source_diagnostics(root)
    else
      {:error, {_line, error, token}} ->
        [
          diagnostic(
            root,
            path,
            1,
            @web_dependency_rule,
            "cannot parse mix.exs: #{error} #{inspect(token)}"
          )
        ]

      {:error, reason} ->
        [
          diagnostic(
            root,
            path,
            1,
            @web_dependency_rule,
            "cannot inspect mix.exs: #{format_error(reason)}"
          )
        ]
    end
  end

  defp git_dependency_options?(options) do
    Enum.any?(options, fn
      keyword when is_list(keyword) ->
        Keyword.has_key?(keyword, :git) or Keyword.has_key?(keyword, :github)

      _option ->
        false
    end)
  end

  defp check_rpc_calls(root) do
    root
    |> elixir_files()
    |> Enum.flat_map(fn path ->
      case parse_elixir(path) do
        {:ok, ast} -> rpc_call_diagnostics(root, path, ast)
        {:error, reason} -> [diagnostic(root, path, 1, @web_rpc_rule, reason)]
      end
    end)
  end

  defp rpc_call_diagnostics(root, path, ast) do
    if rpc_allowed_path?(path) do
      []
    else
      {_ast, calls} =
        Macro.prewalk(ast, [], fn
          {{:., _dot_meta, [{:__aliases__, _alias_meta, parts}, function]}, meta, _arguments} =
              node,
          calls ->
            module = Enum.join(parts, ".")

            if generated_rpc_module?(parts) do
              call = {meta[:line] || 1, "#{module}.#{function}"}
              {node, [call | calls]}
            else
              {node, calls}
            end

          node, calls ->
            {node, calls}
        end)

      calls
      |> Enum.uniq()
      |> Enum.map(fn {line, call} ->
        diagnostic(
          root,
          path,
          line,
          @web_rpc_rule,
          "generated RPC call #{call} is outside the supervised client or page state/effects layer"
        )
      end)
    end
  end

  defp generated_rpc_module?(parts) do
    final = parts |> List.last() |> Atom.to_string()
    Enum.any?(parts, &(&1 == :Generated)) or String.ends_with?(final, ["Client", "Stub"])
  end

  defp rpc_allowed_path?(path) do
    segments = Path.split(path)
    basename = Path.basename(path)

    Enum.any?(["client", "clients", "rpc", "state"], &(&1 in segments)) or
      String.ends_with?(basename, "_state.ex") or
      String.ends_with?(basename, "_effects.ex") or
      basename == "repository_route_model.ex"
  end

  defp infrastructure_source_diagnostics(root) do
    root
    |> application_elixir_files()
    |> Enum.flat_map(fn path ->
      case parse_elixir(path) do
        {:ok, ast} ->
          {_ast, violations} =
            Macro.prewalk(ast, [], fn
              {:__aliases__, meta, parts} = node, violations ->
                module = Enum.join(parts, ".")

                if forbidden_infrastructure_module?(module) do
                  {node, [{meta[:line] || 1, "infrastructure module #{module}"} | violations]}
                else
                  {node, violations}
                end

              {{:., _dot_meta, [{:__aliases__, _alias_meta, [:System]}, function]}, meta,
               [environment | _rest]} = node,
              violations
              when function in [:get_env, :fetch_env] and is_binary(environment) ->
                if environment in @forbidden_infrastructure_env do
                  {node,
                   [
                     {meta[:line] || 1, "infrastructure environment variable #{environment}"}
                     | violations
                   ]}
                else
                  {node, violations}
                end

              literal, violations when is_binary(literal) ->
                if sql_literal?(literal) do
                  {literal, [{1, "SQL literal #{sql_verb(literal)}"} | violations]}
                else
                  {literal, violations}
                end

              node, violations ->
                {node, violations}
            end)

          violations
          |> Enum.uniq()
          |> Enum.map(fn {line, violation} ->
            diagnostic(
              root,
              path,
              line,
              @web_dependency_rule,
              "Phoenix references forbidden #{violation}; use a generated authorized RPC"
            )
          end)

        {:error, reason} ->
          [diagnostic(root, path, 1, @web_dependency_rule, reason)]
      end
    end)
  end

  defp check_backend_clients(root) do
    root
    |> application_elixir_files()
    |> Enum.flat_map(fn path ->
      case parse_elixir(path) do
        {:ok, ast} -> backend_client_diagnostics(root, path, ast)
        {:error, reason} -> [diagnostic(root, path, 1, @web_client_rule, reason)]
      end
    end)
  end

  defp backend_client_diagnostics(root, path, ast) do
    {_ast, calls} =
      Macro.prewalk(ast, [], fn
        {{:., _dot_meta, [module_ast, function]}, meta, _arguments} = node, calls ->
          module = remote_module(module_ast)

          if handwritten_backend_call?(module, function) and
               not backend_client_allowed?(path, module, function) do
            {node, [{meta[:line] || 1, "#{module}.#{function}"} | calls]}
          else
            {node, calls}
          end

        node, calls ->
          {node, calls}
      end)

    calls
    |> Enum.uniq()
    |> Enum.map(fn {line, call} ->
      diagnostic(
        root,
        path,
        line,
        @web_client_rule,
        "hand-written backend call #{call} bypasses generated RPC stubs"
      )
    end)
  end

  defp check_backend_errors(root) do
    root
    |> application_elixir_files()
    |> Enum.flat_map(fn path ->
      case parse_elixir(path) do
        {:ok, ast} -> backend_error_diagnostics(root, path, ast)
        {:error, reason} -> [diagnostic(root, path, 1, @web_error_rule, reason)]
      end
    end)
  end

  defp backend_error_diagnostics(root, path, ast) do
    {_ast, leaks} =
      Macro.prewalk(ast, [], fn
        {function, meta, [argument]} = node, leaks when function in [:inspect, :to_string] ->
          if backend_error_variable?(argument) do
            {node, [{meta[:line] || 1, "#{function}/1 on backend error"} | leaks]}
          else
            {node, leaks}
          end

        {{:., _dot_meta, [{:__aliases__, _alias_meta, [:Exception]}, :message]}, meta, [_error]} =
            node,
        leaks ->
          {node, [{meta[:line] || 1, "Exception.message/1"} | leaks]}

        {{:., _dot_meta, [variable, :message]}, meta, []} = node, leaks ->
          if backend_error_variable?(variable) do
            {node, [{meta[:line] || 1, "direct backend error.message access"} | leaks]}
          else
            {node, leaks}
          end

        node, leaks ->
          {node, leaks}
      end)

    leaks
    |> Enum.uniq()
    |> Enum.map(fn {line, leak} ->
      diagnostic(
        root,
        path,
        line,
        @web_error_rule,
        "#{leak} can expose unrestricted backend text; use HephaestusWeb.RPC.Error.present/1"
      )
    end)
  end

  defp check_filesystem_and_process(root) do
    root
    |> application_elixir_files()
    |> Enum.flat_map(fn path ->
      case parse_elixir(path) do
        {:ok, ast} -> filesystem_and_process_diagnostics(root, path, ast)
        {:error, reason} -> [diagnostic(root, path, 1, @web_io_rule, reason)]
      end
    end)
  end

  defp filesystem_and_process_diagnostics(root, path, ast) do
    {_ast, calls} =
      Macro.prewalk(ast, [], fn
        {{:., _dot_meta, [module_ast, function]}, meta, _arguments} = node, calls ->
          module = remote_module(module_ast)

          if filesystem_or_process_call?(module, function) do
            {node, [{meta[:line] || 1, "#{module}.#{function}"} | calls]}
          else
            {node, calls}
          end

        {:open_port, meta, _arguments} = node, calls ->
          {node, [{meta[:line] || 1, "open_port"} | calls]}

        node, calls ->
          {node, calls}
      end)

    calls
    |> Enum.uniq()
    |> Enum.map(fn {line, call} ->
      diagnostic(
        root,
        path,
        line,
        @web_io_rule,
        "filesystem or subprocess call #{call} is forbidden in Phoenix application code"
      )
    end)
  end

  defp forbidden_infrastructure_module?(module) do
    Enum.any?(@forbidden_infrastructure_modules, fn forbidden ->
      module == forbidden or String.starts_with?(module, forbidden <> ".")
    end)
  end

  defp handwritten_backend_call?(module, function) do
    module in @forbidden_backend_clients or
      module == ":httpc" or
      (module == "GRPC.Stub" and function not in [:connect, :disconnect])
  end

  defp backend_client_allowed?(path, module, function) do
    auth_integration? = String.ends_with?(path, "/controllers/auth_controller.ex")
    rpc_transport? = String.contains?(path, "/lib/hephaestus_web/rpc/")

    (auth_integration? and module in ["Req", "Finch"]) or
      (rpc_transport? and module == "GRPC.Stub" and function in [:connect, :disconnect])
  end

  defp backend_error_variable?({name, _meta, context})
       when is_atom(name) and (is_atom(context) or is_nil(context)) do
    Regex.match?(~r/(?:error|reason|exception)/, Atom.to_string(name))
  end

  defp backend_error_variable?(_argument), do: false

  defp filesystem_or_process_call?(module, function) do
    module in ["File", "File.Stream", "FileSystem", "Port", ":os"] or
      (module == "System" and function in [:cmd, :shell]) or
      (module == ":erlang" and function == :open_port)
  end

  defp remote_module({:__aliases__, _meta, parts}), do: Enum.join(parts, ".")
  defp remote_module(module) when is_atom(module), do: inspect(module)
  defp remote_module(_module), do: ""

  defp sql_literal?(literal) do
    Regex.match?(
      ~r/\A\s*(?:SELECT\b.+\bFROM\b|INSERT\s+INTO\b|UPDATE\s+["\w.]+\s+SET\b|DELETE\s+FROM\b|CREATE\s+(?:TABLE|INDEX|SCHEMA)\b|ALTER\s+TABLE\b|DROP\s+(?:TABLE|INDEX|SCHEMA)\b)/is,
      literal
    )
  end

  defp sql_verb(literal) do
    [verb | _rest] = literal |> String.trim() |> String.split(~r/\s+/, parts: 2)
    String.upcase(verb)
  end

  defp application_elixir_files(root) do
    root
    |> elixir_files()
    |> Enum.reject(fn path ->
      String.contains?(path, "/lib/mix/") or
        String.contains?(path, "/rpc/generated/")
    end)
  end

  defp check_raw_html(root) do
    root
    |> heex_sources()
    |> Enum.reject(fn {path, _source, _line} -> tier(path) == :component end)
    |> Enum.flat_map(fn {path, source, start_line} ->
      raw_tag_diagnostics(root, path, source, start_line)
    end)
  end

  defp raw_tag_diagnostics(root, path, source, start_line) do
    options = [
      file: path,
      line: start_line,
      tag_handler: Phoenix.LiveView.HTMLEngine,
      skip_macro_components: true
    ]

    case Phoenix.LiveView.TagEngine.Parser.parse(source, options) do
      {:ok, parser} ->
        parser.nodes
        |> raw_tags()
        |> Enum.map(fn {name, line} ->
          diagnostic(
            root,
            path,
            line,
            @ui_raw_html_rule,
            "raw <#{name}> tag is outside the basic design-system component tier; " <>
              "render a public component instead"
          )
        end)

      {:error, line, column, reason} ->
        [
          diagnostic(
            root,
            path,
            line,
            @ui_raw_html_rule,
            "cannot parse HEEx at column #{column}: #{reason}"
          )
        ]
    end
  end

  defp raw_tags(nodes) do
    Enum.flat_map(nodes, fn
      {:block, :tag, name, _attrs, children, meta, _close_meta} ->
        [{name, meta[:line] || 1} | raw_tags(children)]

      {:self_close, :tag, name, _attrs, meta} ->
        [{name, meta[:line] || 1}]

      {:eex_block, _expression, clauses, _meta} ->
        Enum.flat_map(clauses, fn {children, _clause, _meta} -> raw_tags(children) end)

      _node ->
        []
    end)
  end

  defp check_tier_direction(root) do
    modules = discover_tier_modules(root)

    module_findings =
      modules
      |> Map.values()
      |> Enum.flat_map(fn %{module: module, path: path, tier: source_tier, ast: ast} ->
        placement_diagnostics(root, module, path, source_tier) ++
          reference_diagnostics(root, module, path, source_tier, ast, modules) ++
          heex_tier_diagnostics(root, module, path, source_tier)
      end)

    module_findings ++ composite_cycle_diagnostics(root, modules)
  end

  defp check_page_companions(root) do
    root
    |> live_view_files()
    |> Enum.flat_map(fn path ->
      stem = path |> Path.basename(".ex") |> String.trim_trailing("_live")

      with {:ok, ast} <- parse_elixir(path),
           live_module when is_binary(live_module) <- defined_module(ast) do
        state_module = sibling_module(live_module, Macro.camelize(stem) <> "State")

        page_module =
          sibling_module(live_module, "DesignSystem.Pages.#{Macro.camelize(stem)}Page")

        adapter_findings =
          if String.ends_with?(live_module, ".#{Macro.camelize(stem)}Live") or
               live_module == Macro.camelize(stem) <> "Live" do
            []
          else
            [
              diagnostic(
                root,
                path,
                1,
                @ui_companions_rule,
                "route adapter module #{live_module} does not match #{Path.basename(path)}"
              )
            ]
          end

        companions = [
          {"state/effects module", Path.join(Path.dirname(path), "#{stem}_state.ex"),
           state_module, :exact},
          {"page component", Path.join(root, "lib/**/design_system/pages/#{stem}_page.ex"),
           page_module, :glob},
          {"state test", Path.join(root, "test/**/#{stem}_state_test.exs"),
           Macro.camelize(stem) <> "StateTest", :glob},
          {"page render test", Path.join(root, "test/**/#{stem}_page_test.exs"),
           Macro.camelize(stem) <> "PageTest", :glob}
        ]

        adapter_findings ++
          Enum.flat_map(companions, fn {label, location, expected_module, kind} ->
            companion_diagnostics(
              root,
              path,
              stem,
              label,
              location,
              expected_module,
              kind
            )
          end)
      else
        nil ->
          [diagnostic(root, path, 1, @ui_companions_rule, "route adapter has no module")]

        {:error, reason} ->
          [diagnostic(root, path, 1, @ui_companions_rule, reason)]
      end
    end)
  end

  defp check_live_renders(root) do
    root
    |> live_view_files()
    |> Enum.flat_map(fn path ->
      with {:ok, ast} <- parse_elixir(path),
           live_module when is_binary(live_module) <- defined_module(ast) do
        stem = path |> Path.basename(".ex") |> String.trim_trailing("_live")

        expected_page =
          sibling_module(live_module, "DesignSystem.Pages.#{Macro.camelize(stem)}Page")

        page_tags =
          path
          |> file_heex_sources()
          |> Enum.flat_map(fn {_path, source, start_line} ->
            heex_page_components(path, source, start_line)
          end)

        page_count_findings =
          if length(page_tags) == 1 do
            [{tag, line}] = page_tags

            if resolve_component_module(tag, alias_map(ast)) == expected_page do
              []
            else
              [
                diagnostic(
                  root,
                  path,
                  line,
                  @ui_one_page_rule,
                  "LiveView render must invoke exactly one component from #{expected_page}; found <#{tag}>"
                )
              ]
            end
          else
            diagnostic_line =
              page_tags |> List.first() |> then(&if(&1, do: elem(&1, 1), else: 1))

            [
              diagnostic(
                root,
                path,
                diagnostic_line,
                @ui_one_page_rule,
                "LiveView render must invoke exactly one page component; found #{length(page_tags)}"
              )
            ]
          end

        page_count_findings ++ adapter_contract_diagnostics(root, path, ast, live_module)
      else
        nil -> [diagnostic(root, path, 1, @ui_one_page_rule, "route adapter has no module")]
        {:error, reason} -> [diagnostic(root, path, 1, @ui_one_page_rule, reason)]
      end
    end)
  end

  defp check_state_modules(root) do
    root
    |> Path.join("lib/**/*_state.ex")
    |> Path.wildcard()
    |> Enum.reject(&design_system_rendering_path?/1)
    |> Enum.flat_map(fn path ->
      with {:ok, ast} <- parse_elixir(path) do
        own_module = defined_module(ast)

        {_ast, concerns} =
          Macro.prewalk(ast, [], fn
            {:sigil_H, meta, _arguments} = node, concerns ->
              {node, [{meta[:line] || 1, "HEEx rendering"} | concerns]}

            {:__aliases__, meta, parts} = node, concerns ->
              referenced_module = Enum.join(parts, ".")

              if referenced_module != own_module and
                   forbidden_state_reference?(referenced_module) do
                {node,
                 [
                   {meta[:line] || 1, "rendering/runtime reference #{referenced_module}"}
                   | concerns
                 ]}
              else
                {node, concerns}
              end

            {:socket, meta, _context} = node, concerns ->
              {node, [{meta[:line] || 1, "socket access"} | concerns]}

            {{:., _dot_meta, [module, function]}, meta, _arguments} = node, concerns
            when module in [:ets, :persistent_term] ->
              {node,
               [
                 {meta[:line] || 1, "page-local runtime call #{module}.#{function}"}
                 | concerns
               ]}

            node, concerns ->
              {node, concerns}
          end)

        concern_findings =
          concerns
          |> Enum.uniq()
          |> Enum.map(fn {line, concern} ->
            diagnostic(
              root,
              path,
              line,
              @ui_state_rule,
              "state/effects module contains #{concern}; construct a presentation model without rendering concerns"
            )
          end)

        concern_findings ++ state_contract_diagnostics(root, path, ast)
      else
        {:error, reason} -> [diagnostic(root, path, 1, @ui_state_rule, reason)]
      end
    end)
  end

  defp check_pure_pages(root) do
    root
    |> Path.join("lib/**/*_page.ex")
    |> Path.wildcard()
    |> Enum.flat_map(fn path ->
      with {:ok, ast} <- parse_elixir(path) do
        own_module = defined_module(ast)

        {_ast, concerns} =
          Macro.prewalk(ast, [], fn
            {definition, meta, [{name, _function_meta, _arguments} | _rest]} = node, concerns
            when definition in [:def, :defp] and
                   name in [
                     :mount,
                     :handle_event,
                     :handle_info,
                     :handle_params,
                     :handle_async,
                     :terminate
                   ] ->
              {node, [{meta[:line] || 1, "LiveView callback #{name}"} | concerns]}

            {:__aliases__, meta, parts} = node, concerns ->
              reference = Enum.join(parts, ".")

              if reference != own_module and forbidden_page_reference?(reference) do
                {node, [{meta[:line] || 1, "runtime dependency #{reference}"} | concerns]}
              else
                {node, concerns}
              end

            {:socket, meta, _context} = node, concerns ->
              {node, [{meta[:line] || 1, "socket access"} | concerns]}

            {{:., _dot_meta, [module, function]}, meta, _arguments} = node, concerns
            when module in [:ets, :persistent_term] ->
              {node,
               [
                 {meta[:line] || 1, "mutable runtime call #{module}.#{function}"}
                 | concerns
               ]}

            {function, meta, arguments} = node, concerns
            when function in [
                   :put_flash,
                   :push_event,
                   :push_navigate,
                   :push_patch,
                   :send,
                   :send_after,
                   :spawn,
                   :spawn_link,
                   :start_async
                 ] and is_list(arguments) ->
              {node, [{meta[:line] || 1, "runtime call #{function}"} | concerns]}

            node, concerns ->
              {node, concerns}
          end)

        concerns
        |> Enum.uniq()
        |> Enum.map(fn {line, concern} ->
          diagnostic(
            root,
            path,
            line,
            @ui_pure_page_rule,
            "page component contains #{concern}; pages must be pure presentation functions"
          )
        end)
      else
        {:error, reason} -> [diagnostic(root, path, 1, @ui_pure_page_rule, reason)]
      end
    end)
  end

  defp companion_diagnostics(
         root,
         adapter_path,
         stem,
         label,
         location,
         expected_module,
         kind
       ) do
    paths =
      if kind == :exact,
        do: Enum.filter([location], &File.regular?/1),
        else: Path.wildcard(location)

    case paths do
      [] ->
        [
          diagnostic(
            root,
            adapter_path,
            1,
            @ui_companions_rule,
            "LiveView #{Path.basename(adapter_path)} is missing its #{label} companion for #{stem}"
          )
        ]

      [path] ->
        case parse_elixir(path) do
          {:ok, ast} ->
            module = defined_module(ast)

            if companion_module_matches?(module, expected_module) do
              companion_coverage_diagnostics(root, path, label, ast)
            else
              [
                diagnostic(
                  root,
                  path,
                  1,
                  @ui_companions_rule,
                  "#{label} must define #{expected_module}; found #{inspect(module)}"
                )
              ]
            end

          {:error, reason} ->
            [diagnostic(root, path, 1, @ui_companions_rule, reason)]
        end

      paths ->
        [
          diagnostic(
            root,
            adapter_path,
            1,
            @ui_companions_rule,
            "LiveView #{Path.basename(adapter_path)} has #{length(paths)} #{label} companions; exactly one is required"
          )
        ]
    end
  end

  defp companion_module_matches?(module, expected) when is_binary(module) do
    if String.contains?(expected, ".") do
      module == expected
    else
      List.last(String.split(module, ".")) == expected
    end
  end

  defp companion_module_matches?(_module, _expected), do: false

  defp companion_coverage_diagnostics(root, path, "state test", ast) do
    covered = module_attribute_literal(ast, :covered_statuses)

    if complete_status_coverage?(covered) do
      []
    else
      [
        diagnostic(
          root,
          path,
          1,
          @ui_companions_rule,
          "state test must declare exact @covered_statuses #{inspect(@page_statuses)}"
        )
      ]
    end
  end

  defp companion_coverage_diagnostics(root, path, "page render test", ast) do
    covered_statuses = module_attribute_literal(ast, :covered_statuses)
    mapping = module_attribute_literal(ast, :status_visual_states)
    covered_states = module_attribute_literal(ast, :covered_states)

    direct_coverage? = complete_status_coverage?(covered_statuses)

    mapped_coverage? =
      is_map(mapping) and MapSet.new(Map.keys(mapping)) == MapSet.new(@page_statuses) and
        atom_list?(Map.values(mapping)) and atom_list?(covered_states) and
        MapSet.new(Map.values(mapping)) == MapSet.new(covered_states)

    if direct_coverage? or mapped_coverage? do
      []
    else
      [
        diagnostic(
          root,
          path,
          1,
          @ui_companions_rule,
          "page render test must cover all eight @covered_statuses or declare an exact @status_visual_states mapping to @covered_states"
        )
      ]
    end
  end

  defp companion_coverage_diagnostics(_root, _path, _label, _ast), do: []

  defp complete_status_coverage?(statuses) do
    atom_list?(statuses) and length(statuses) == length(@page_statuses) and
      MapSet.new(statuses) == MapSet.new(@page_statuses)
  end

  defp sibling_module(module, replacement) do
    module
    |> String.split(".")
    |> Enum.drop(-1)
    |> Kernel.++(String.split(replacement, "."))
    |> Enum.join(".")
  end

  defp resolve_component_module(tag, aliases) do
    module_parts = tag |> String.split(".") |> Enum.drop(-1)

    case module_parts do
      [first | rest] ->
        [Map.get(aliases, first, first) | rest] |> Enum.join(".")

      [] ->
        ""
    end
  end

  defp referenced_live_support_asts(root, ast) do
    aliases = alias_map(ast)

    references =
      ast
      |> module_references()
      |> MapSet.new(fn {_line, reference} -> Map.get(aliases, reference, reference) end)

    support_paths =
      ["lib/**/*_live_support.ex", "lib/**/page_stream.ex"]
      |> Enum.flat_map(fn pattern -> root |> Path.join(pattern) |> Path.wildcard() end)
      |> Enum.uniq()

    support_paths
    |> Enum.flat_map(fn path ->
      with {:ok, support_ast} <- parse_elixir(path),
           module when is_binary(module) <- defined_module(support_ast),
           short_module = List.last(String.split(module, ".")),
           true <-
             MapSet.member?(references, module) or MapSet.member?(references, short_module) do
        [support_ast]
      else
        _other -> []
      end
    end)
  end

  defp adapter_contract_diagnostics(root, path, ast, live_module) do
    functions = defined_functions(ast)
    function_set = MapSet.new(functions, &{&1.name, &1.arity})
    contract_asts = [ast | referenced_live_support_asts(root, ast)]

    callback_findings =
      functions
      |> Enum.filter(&(&1.visibility == :public))
      |> Enum.reject(&({&1.name, &1.arity} in @live_callbacks))
      |> Enum.map(fn function ->
        diagnostic(
          root,
          path,
          function.line,
          @ui_one_page_rule,
          "route adapter exposes non-callback #{function.name}/#{function.arity}; move product logic to its state/effects module"
        )
      end)

    stream_mode = module_attribute_literal(ast, :stream_mode)

    stream_mode_findings =
      if stream_mode in [:none, :page_scoped] do
        []
      else
        [
          diagnostic(
            root,
            path,
            1,
            @ui_one_page_rule,
            "route adapter must declare literal @stream_mode :none or :page_scoped"
          )
        ]
      end

    state_assignment_findings =
      if contract_asts
         |> Enum.flat_map(&assigned_socket_keys/1)
         |> Enum.any?(&(&1 == :page_state)) do
        []
      else
        [
          diagnostic(
            root,
            path,
            1,
            @ui_one_page_rule,
            "route adapter must assign its state struct under socket key :page_state"
          )
        ]
      end

    sensitive_findings =
      contract_asts
      |> Enum.flat_map(&assigned_socket_keys/1)
      |> Enum.filter(&sensitive_state_name?/1)
      |> Enum.uniq()
      |> Enum.map(fn key ->
        diagnostic(
          root,
          path,
          1,
          @ui_one_page_rule,
          "route adapter assigns sensitive field #{inspect(key)} to the socket; plaintext must remain transient"
        )
      end)

    dependency_findings =
      ast
      |> module_references()
      |> Enum.reject(fn {_line, reference} -> reference == live_module end)
      |> Enum.filter(fn {_line, reference} -> forbidden_adapter_reference?(reference) end)
      |> Enum.map(fn {line, reference} ->
        diagnostic(
          root,
          path,
          line,
          @ui_one_page_rule,
          "route adapter references product client/service #{reference}; execute effects in its state module"
        )
      end)

    runtime_findings =
      contract_asts
      |> Enum.flat_map(&remote_calls/1)
      |> Enum.filter(fn
        {module, _function} when module in ["ets", "persistent_term"] ->
          true

        {"Process", function} when function in [:put, :get, :register, :unregister, :whereis] ->
          true

        _call ->
          false
      end)
      |> Enum.map(fn {module, function} ->
        diagnostic(
          root,
          path,
          1,
          @ui_one_page_rule,
          "route adapter uses page-local runtime #{module}.#{function}; keep page state only in the LiveView socket"
        )
      end)

    stream_findings =
      if stream_mode == :page_scoped do
        calls = Enum.flat_map(contract_asts, &remote_calls/1)
        atoms = contract_asts |> Enum.map(&ast_atoms/1) |> Enum.reduce(&MapSet.union/2)

        requirements = [
          {MapSet.member?(function_set, {:handle_info, 2}),
           "define handle_info/2 for tagged delivery"},
          {MapSet.member?(function_set, {:terminate, 2}),
           "define terminate/2 to cancel the stream"},
          {Enum.any?(calls, fn {module, _function} -> module == "Task.Supervisor" end),
           "start the stream below Task.Supervisor"},
          {Enum.any?(calls, fn
             {"Task", function} -> function in [:shutdown, :yield_many]
             {"Task.Supervisor", :terminate_child} -> true
             _call -> false
           end), "cancel the supervised task on replacement and termination"},
          {:cursor in atoms, "resume from the committed cursor"},
          {:stream_generation in atoms, "tag and reject stale stream generations"}
        ]

        requirements
        |> Enum.reject(&elem(&1, 0))
        |> Enum.map(fn {_satisfied?, requirement} ->
          diagnostic(
            root,
            path,
            1,
            @ui_one_page_rule,
            "page-scoped stream must #{requirement}"
          )
        end)
      else
        []
      end

    callback_findings ++
      stream_mode_findings ++
      state_assignment_findings ++
      sensitive_findings ++ dependency_findings ++ runtime_findings ++ stream_findings
  end

  defp state_contract_diagnostics(root, path, ast) do
    statuses = module_attribute_literal(ast, :statuses)
    fields = defstruct_fields(ast)
    functions = MapSet.new(defined_functions(ast), &{&1.name, &1.arity})

    placement_findings =
      if path |> String.replace("\\", "/") |> String.contains?("/live/") do
        []
      else
        [
          diagnostic(
            root,
            path,
            1,
            @ui_state_rule,
            "state/effects module must be a sibling of its route adapter under lib/**/live"
          )
        ]
      end

    status_findings =
      if statuses == @page_statuses do
        []
      else
        [
          diagnostic(
            root,
            path,
            1,
            @ui_state_rule,
            "state/effects module must declare exact literal @statuses #{inspect(@page_statuses)}"
          )
        ]
      end

    standard_fields? =
      MapSet.new(Map.keys(fields)) == MapSet.new(@page_state_field_names) and
        fields[:status] == :initial and fields[:error] == nil and fields[:cursor] == nil and
        fields[:stream_generation] == 0 and (is_nil(fields[:form]) or is_map(fields[:form]))

    struct_findings =
      if standard_fields? do
        []
      else
        sensitive_fields =
          fields
          |> Map.keys()
          |> Enum.filter(&sensitive_state_name?/1)

        base =
          diagnostic(
            root,
            path,
            1,
            @ui_state_rule,
            "state/effects defstruct must use exactly status/data/form/error/cursor/stream_generation and the standard lifecycle defaults"
          )

        secret_findings =
          Enum.map(sensitive_fields, fn field ->
            diagnostic(
              root,
              path,
              1,
              @ui_state_rule,
              "state/effects struct retains sensitive field #{inspect(field)}; plaintext must remain transient"
            )
          end)

        [base | secret_findings]
      end

    api_findings =
      [statuses: 0, new: 1, reduce: 2, execute: 2, present: 1]
      |> Enum.reject(&MapSet.member?(functions, &1))
      |> Enum.map(fn {name, arity} ->
        diagnostic(
          root,
          path,
          1,
          @ui_state_rule,
          "state/effects module must define #{name}/#{arity}"
        )
      end)

    placement_findings ++ status_findings ++ struct_findings ++ api_findings
  end

  defp forbidden_state_reference?(reference) do
    String.contains?(reference, [
      ".DesignSystem",
      ".Components.",
      ".Composites.",
      ".Pages.",
      "Phoenix.Component",
      "Phoenix.LiveView"
    ]) or
      reference in ["GenServer", "Agent", "Registry", "Task", "Process", "File", "System", "Port"]
  end

  defp forbidden_adapter_reference?(reference) do
    generated_rpc_reference?(reference) or
      String.ends_with?(reference, [
        "Client",
        "Stub",
        "Service",
        "Store",
        "Repo",
        "Repository",
        "Notifier"
      ]) or reference in ["GenServer", "Agent", "Registry"]
  end

  defp forbidden_page_reference?(reference) do
    cond do
      String.contains?(reference, ".DesignSystem") ->
        false

      reference == "Phoenix.Component" ->
        false

      generated_rpc_reference?(reference) ->
        true

      String.starts_with?(reference, "Phoenix.LiveView") ->
        true

      String.contains?(reference, [".Domain.", ".Application."]) ->
        true

      String.ends_with?(reference, [
        "Client",
        "Stub",
        "Service",
        "Store",
        "Repo",
        "Repository",
        "State",
        "Live",
        "Router",
        "Endpoint",
        "Notifier"
      ]) ->
        true

      reference in [
        "GenServer",
        "Agent",
        "Registry",
        "Task",
        "Process",
        "File",
        "System",
        "Port"
      ] ->
        true

      true ->
        false
    end
  end

  defp generated_rpc_reference?(reference) do
    String.contains?(reference, [".Generated.", ".Protobuf.", ".Proto.", ".Rpc.", ".RPC."]) or
      String.starts_with?(reference, "HephaestusWebWeb.Generated.")
  end

  defp sensitive_state_name?(name) when is_atom(name),
    do: name |> Atom.to_string() |> sensitive_state_name?()

  defp sensitive_state_name?(name) when is_binary(name) do
    Regex.match?(
      ~r/(^|_)(password|passphrase|plaintext|private_key|access_token|refresh_token|secret_value|secret_form|secret_token|credential|credentials|token_value)($|_)/,
      name
    )
  end

  defp sensitive_state_name?(_name), do: false

  defp assigned_socket_keys(ast) do
    {_ast, keys} =
      Macro.prewalk(ast, [], fn
        {:assign, _meta, arguments} = node, keys when is_list(arguments) ->
          key =
            case arguments do
              [_socket, key | _rest] when is_atom(key) -> key
              [key | _rest] when is_atom(key) -> key
              _arguments -> nil
            end

          {node, if(is_nil(key), do: keys, else: [key | keys])}

        node, keys ->
          {node, keys}
      end)

    keys
  end

  defp defstruct_fields(ast) do
    aliases = alias_map(ast)

    {_ast, fields} =
      Macro.prewalk(ast, %{}, fn
        {:defstruct, _meta, [fields_ast]} = node, fields when fields == %{} ->
          decoded =
            case literal_value(fields_ast, aliases) do
              {:ok, values} when is_list(values) -> Map.new(values)
              _other -> %{}
            end

          {node, decoded}

        node, fields ->
          {node, fields}
      end)

    fields
  end

  defp defined_functions(ast) do
    {_ast, functions} =
      Macro.prewalk(ast, [], fn
        {visibility, meta, [head | _rest]} = node, functions when visibility in [:def, :defp] ->
          case function_name_and_arity(head) do
            {name, arity} ->
              function = %{
                name: name,
                arity: arity,
                visibility: if(visibility == :def, do: :public, else: :private),
                line: meta[:line] || 1
              }

              {node, [function | functions]}

            nil ->
              {node, functions}
          end

        node, functions ->
          {node, functions}
      end)

    Enum.reverse(functions)
  end

  defp function_name_and_arity({:when, _meta, [head | _guards]}),
    do: function_name_and_arity(head)

  defp function_name_and_arity({name, _meta, arguments})
       when is_atom(name) and is_list(arguments),
       do: {name, length(arguments)}

  defp function_name_and_arity({name, _meta, nil}) when is_atom(name), do: {name, 0}
  defp function_name_and_arity(_head), do: nil

  defp remote_calls(ast) do
    aliases = alias_map(ast)

    {_ast, calls} =
      Macro.prewalk(ast, MapSet.new(), fn
        {{:., _dot_meta, [{:__aliases__, _alias_meta, parts}, function]}, _meta, _arguments} =
            node,
        calls ->
          module = Enum.join(parts, ".")
          resolved = Map.get(aliases, module, module)
          {node, MapSet.put(calls, {resolved, function})}

        {{:., _dot_meta, [module, function]}, _meta, _arguments} = node, calls
        when is_atom(module) ->
          {node, MapSet.put(calls, {Atom.to_string(module), function})}

        node, calls ->
          {node, calls}
      end)

    calls
  end

  defp ast_atoms(ast) do
    {_ast, atoms} =
      Macro.prewalk(ast, MapSet.new(), fn
        atom, atoms when is_atom(atom) -> {atom, MapSet.put(atoms, atom)}
        node, atoms -> {node, atoms}
      end)

    atoms
  end

  @interaction_attributes ~w(
    phx-click phx-submit phx-change phx-blur phx-focus phx-keydown phx-keyup
    phx-window-keydown phx-window-keyup phx-hook
  )

  @interaction_property_names [
    :action,
    :event,
    :interaction,
    :on_blur,
    :on_change,
    :on_click,
    :on_focus,
    :on_keydown,
    :on_keyup,
    :on_submit
  ]

  defp check_declared_interactions(root) do
    property_findings =
      root
      |> ui_elixir_files()
      |> Enum.flat_map(fn path ->
        with {:ok, ast} <- parse_elixir(path) do
          ast
          |> component_attribute_declarations()
          |> Enum.filter(fn %{name: name} -> interaction_property_name?(name) end)
          |> Enum.reject(&bounded_interaction_attribute?/1)
          |> Enum.map(fn declaration ->
            diagnostic(
              root,
              path,
              declaration.line,
              @ui_interaction_rule,
              "interaction property :#{declaration.name} is not bounded by literal values; " <>
                "declare the supported interaction vocabulary explicitly"
            )
          end)
        else
          {:error, reason} -> [diagnostic(root, path, 1, @ui_interaction_rule, reason)]
        end
      end)

    heex_findings =
      root
      |> heex_sources()
      |> Enum.reject(fn {path, _source, _line} -> tier(path) == :component end)
      |> Enum.flat_map(fn {path, source, start_line} ->
        case parse_heex(path, source, start_line) do
          {:ok, nodes} ->
            nodes
            |> heex_attributes()
            |> Enum.filter(fn %{name: name} ->
              name in @interaction_attributes or String.starts_with?(name, "phx-value-")
            end)
            |> Enum.reject(&declared_interaction_expression?/1)
            |> Enum.map(fn attribute ->
              diagnostic(
                root,
                path,
                attribute.line,
                @ui_interaction_rule,
                "interaction attribute #{attribute.name} is a scattered literal or expression; " <>
                  "pass a declared interaction property from the presentation model"
              )
            end)

          {:error, line, column, reason} ->
            [
              diagnostic(
                root,
                path,
                line,
                @ui_interaction_rule,
                "cannot parse HEEx at column #{column}: #{reason}"
              )
            ]
        end
      end)

    property_findings ++ heex_findings
  end

  @unrestricted_style_properties [:class, :error_class, :style]
  @layout_property_names [
    :column_widths,
    :columns,
    :gap,
    :grid,
    :height,
    :layout,
    :margin,
    :padding,
    :spacing,
    :width
  ]

  defp check_class_escape_hatches(root) do
    declaration_findings =
      root
      |> ui_elixir_files()
      |> Enum.flat_map(fn path ->
        with {:ok, ast} <- parse_elixir(path) do
          ast
          |> component_attribute_declarations()
          |> Enum.flat_map(fn declaration ->
            cond do
              declaration.name in @unrestricted_style_properties ->
                [
                  diagnostic(
                    root,
                    path,
                    declaration.line,
                    @ui_class_rule,
                    "public property :#{declaration.name} is an unrestricted styling escape hatch; " <>
                      "replace it with bounded design-system properties"
                  )
                ]

              declaration.type == :global ->
                [
                  diagnostic(
                    root,
                    path,
                    declaration.line,
                    @ui_class_rule,
                    "global property :#{declaration.name} admits class/style attributes; " <>
                      "declare each supported HTML attribute explicitly"
                  )
                ]

              declaration.name in @layout_property_names and not bounded_attribute?(declaration) ->
                [
                  diagnostic(
                    root,
                    path,
                    declaration.line,
                    @ui_class_rule,
                    "layout property :#{declaration.name} is not bounded by literal values; " <>
                      "replace arbitrary layout input with a finite design-system property"
                  )
                ]

              true ->
                []
            end
          end)
        else
          {:error, reason} -> [diagnostic(root, path, 1, @ui_class_rule, reason)]
        end
      end)

    heex_findings =
      root
      |> heex_sources()
      |> Enum.reject(fn {path, _source, _line} -> tier(path) == :component end)
      |> Enum.flat_map(fn {path, source, start_line} ->
        case parse_heex(path, source, start_line) do
          {:ok, nodes} ->
            nodes
            |> heex_attributes()
            |> Enum.filter(&(&1.name in ["class", "style"]))
            |> Enum.map(fn attribute ->
              diagnostic(
                root,
                path,
                attribute.line,
                @ui_class_rule,
                "#{attribute.name} is authored outside the basic component tier; " <>
                  "select a bounded facade property instead"
              )
            end)

          {:error, line, column, reason} ->
            [
              diagnostic(
                root,
                path,
                line,
                @ui_class_rule,
                "cannot parse HEEx at column #{column}: #{reason}"
              )
            ]
        end
      end)

    declaration_findings ++ heex_findings
  end

  defp check_design_tokens(root) do
    css_findings =
      root
      |> Path.join("assets/css/**/*.css")
      |> Path.wildcard()
      |> Enum.reject(&design_system_css_path?/1)
      |> Enum.flat_map(&css_token_diagnostics(root, &1))

    heex_findings =
      root
      |> heex_sources()
      |> Enum.reject(fn {path, _source, _line} -> tier(path) == :component end)
      |> Enum.flat_map(fn {path, source, start_line} ->
        case parse_heex(path, source, start_line) do
          {:ok, nodes} ->
            nodes
            |> heex_attributes()
            |> Enum.filter(&(&1.name in ["class", "style"]))
            |> Enum.map(fn attribute ->
              diagnostic(
                root,
                path,
                attribute.line,
                @ui_token_rule,
                "#{attribute.name} bypasses centralized design tokens outside the basic component tier"
              )
            end)

          {:error, line, column, reason} ->
            [
              diagnostic(
                root,
                path,
                line,
                @ui_token_rule,
                "cannot parse HEEx at column #{column}: #{reason}"
              )
            ]
        end
      end)

    css_findings ++ heex_findings
  end

  defp css_token_diagnostics(root, path) do
    case File.read(path) do
      {:ok, source} ->
        token_patterns = [
          {~r/(?:#(?:[0-9a-fA-F]{3,8})|\b(?:rgb|hsl|oklch|lab|color)\s*\()/, "literal color"},
          {~r/\bfont(?:-family)?\s*:\s*(?!var\()/, "literal font"},
          {~r/\bborder(?:-[a-z]+)*-radius\s*:\s*(?!var\()/, "literal radius"},
          {~r/\bbox-shadow\s*:\s*(?!var\()/, "literal shadow"},
          {~r/\b(?:gap|margin(?:-[a-z]+)?|padding(?:-[a-z]+)?|inset|top|right|bottom|left)\s*:\s*(?!0(?:\D|$)|var\()/,
           "unapproved spacing"}
        ]

        source
        |> String.split("\n")
        |> Enum.with_index(1)
        |> Enum.flat_map(fn {line_source, line} ->
          token_patterns
          |> Enum.filter(fn {pattern, _description} -> Regex.match?(pattern, line_source) end)
          |> Enum.map(fn {_pattern, description} ->
            diagnostic(
              root,
              path,
              line,
              @ui_token_rule,
              "CSS contains #{description} outside assets/css/design_system; " <>
                "define the value once as a design token"
            )
          end)
        end)

      {:error, reason} ->
        [diagnostic(root, path, 1, @ui_token_rule, "cannot inspect CSS: #{format_error(reason)}")]
    end
  end

  defp check_external_ui_imports(root) do
    elixir_findings =
      root
      |> elixir_files()
      |> Enum.reject(&(tier(&1) == :component))
      |> Enum.flat_map(fn path ->
        with {:ok, ast} <- parse_elixir(path) do
          ast
          |> module_references()
          |> Enum.filter(fn {_line, reference} -> external_ui_module?(reference) end)
          |> Enum.map(fn {line, reference} ->
            diagnostic(
              root,
              path,
              line,
              @ui_external_rule,
              "external UI implementation #{reference} is used outside the basic component tier; " <>
                "wrap it behind the public design-system facade"
            )
          end)
        else
          {:error, reason} -> [diagnostic(root, path, 1, @ui_external_rule, reason)]
        end
      end)

    css_findings =
      root
      |> Path.join("assets/css/**/*.css")
      |> Path.wildcard()
      |> Enum.reject(&design_system_css_path?/1)
      |> Enum.flat_map(fn path ->
        case File.read(path) do
          {:ok, source} ->
            source
            |> String.split("\n")
            |> Enum.with_index(1)
            |> Enum.filter(fn {line, _number} -> Regex.match?(~r/^\s*@plugin\b/, line) end)
            |> Enum.map(fn {_line, number} ->
              diagnostic(
                root,
                path,
                number,
                @ui_external_rule,
                "CSS plugin is imported outside assets/css/design_system"
              )
            end)

          {:error, reason} ->
            [
              diagnostic(
                root,
                path,
                1,
                @ui_external_rule,
                "cannot inspect CSS: #{format_error(reason)}"
              )
            ]
        end
      end)

    javascript_findings =
      root
      |> Path.join("assets/js/**/*.{js,ts}")
      |> Path.wildcard()
      |> Enum.reject(&design_system_javascript_path?/1)
      |> Enum.flat_map(&javascript_import_diagnostics(root, &1))

    elixir_findings ++ css_findings ++ javascript_findings
  end

  defp check_dom_injection(root) do
    root
    |> Path.join("assets/js/**/*.{js,ts}")
    |> Path.wildcard()
    |> Enum.reject(&design_system_hook_path?/1)
    |> Enum.flat_map(fn path ->
      case File.read(path) do
        {:ok, source} ->
          dom_injection_diagnostics(root, path, source)

        {:error, reason} ->
          [
            diagnostic(
              root,
              path,
              1,
              @ui_dom_injection_rule,
              "cannot inspect JavaScript source: #{format_error(reason)}"
            )
          ]
      end
    end)
  end

  defp dom_injection_diagnostics(root, path, source) do
    patterns = [
      {~r/\binnerHTML\b/, "innerHTML use"},
      {~r/\bouterHTML\b/, "outerHTML use"},
      {~r/\binsertAdjacentHTML\s*\(/, "insertAdjacentHTML call"},
      {~r/\b(?:document\s*\.\s*)?write(?:ln)?\s*\(/, "document.write call"},
      {~r/\bcreateElement(?:NS)?\s*\(/, "raw DOM creation"},
      {~r/\bDOMParser\b/, "DOMParser construction"},
      {~r/\bparseFromString\s*\(/, "markup parsing"},
      {~r/\bcreateContextualFragment\s*\(/, "contextual fragment creation"},
      {~r/\bsetHTMLUnsafe\s*\(/, "unsafe HTML insertion"},
      {~r/\bdangerouslySetInnerHTML\b/, "dangerous inner HTML property"},
      {~r/\.\s*(?:html|append)\s*\(/, "library markup insertion"}
    ]

    source
    |> javascript_code_only()
    |> String.split("\n")
    |> Enum.with_index(1)
    |> Enum.flat_map(fn {line_source, line} ->
      patterns
      |> Enum.filter(fn {pattern, _description} -> Regex.match?(pattern, line_source) end)
      |> Enum.map(fn {_pattern, description} ->
        diagnostic(
          root,
          path,
          line,
          @ui_dom_injection_rule,
          "JavaScript uses #{description} outside the designated design-system hooks; " <>
            "render application markup through HEEx components"
        )
      end)
    end)
  end

  @catalog_keys [
    :name,
    :tier,
    :module,
    :function,
    :attrs,
    :slots,
    :showcase_id,
    :a11y_test_id
  ]

  defp check_public_facade(root) do
    case facade_catalog(root) do
      {:error, path, message} ->
        [diagnostic(root, path, 1, @ui_facade_rule, message)]

      {:ok, path, ast, entries} ->
        catalog_shape_diagnostics(root, path, entries) ++
          facade_delegate_diagnostics(root, path, ast, entries) ++
          implementation_contract_diagnostics(root, entries)
    end
  end

  defp catalog_shape_diagnostics(root, path, entries) do
    entry_findings =
      entries
      |> Enum.with_index(1)
      |> Enum.flat_map(fn {entry, index} ->
        cond do
          not is_map(entry) ->
            [
              diagnostic(
                root,
                path,
                1,
                @ui_facade_rule,
                "catalog entry #{index} must be a literal map"
              )
            ]

          missing = @catalog_keys -- Map.keys(entry) ->
            if missing == [] do
              validate_catalog_entry(root, path, entry)
            else
              [
                diagnostic(
                  root,
                  path,
                  1,
                  @ui_facade_rule,
                  "catalog entry #{index} is missing #{Enum.map_join(missing, ", ", &inspect/1)}"
                )
              ]
            end
        end
      end)

    duplicate_findings =
      [:name, :showcase_id, :a11y_test_id]
      |> Enum.flat_map(fn key ->
        entries
        |> Enum.filter(&is_map/1)
        |> Enum.map(&Map.get(&1, key))
        |> Enum.reject(&is_nil/1)
        |> Enum.map(&normalize_identifier/1)
        |> Enum.frequencies()
        |> Enum.filter(fn {_value, count} -> count > 1 end)
        |> Enum.map(fn {value, _count} ->
          diagnostic(
            root,
            path,
            1,
            @ui_facade_rule,
            "catalog #{key} #{inspect(value)} is duplicated"
          )
        end)
      end)

    entry_findings ++ duplicate_findings
  end

  defp validate_catalog_entry(root, path, entry) do
    validations = [
      {is_atom(entry.name), ":name must be an atom"},
      {entry.tier in [:component, :composite], ":tier must be :component or :composite"},
      {is_binary(entry.module), ":module must be a module alias"},
      {is_atom(entry.function), ":function must be an atom"},
      {atom_list?(entry.attrs), ":attrs must be a literal atom list"},
      {atom_list?(entry.slots), ":slots must be a literal atom list"},
      {identifier?(entry.showcase_id), ":showcase_id must be an atom or non-empty string"},
      {identifier?(entry.a11y_test_id), ":a11y_test_id must be an atom or non-empty string"}
    ]

    validations
    |> Enum.reject(&elem(&1, 0))
    |> Enum.map(fn {_valid?, message} ->
      diagnostic(
        root,
        path,
        1,
        @ui_facade_rule,
        "catalog entry #{inspect(entry[:name])} #{message}"
      )
    end)
  end

  defp facade_delegate_diagnostics(root, path, ast, entries) do
    aliases = alias_map(ast)
    delegates = facade_delegates(ast, aliases)

    entries
    |> Enum.filter(&valid_catalog_entry?/1)
    |> Enum.flat_map(fn entry ->
      case Map.get(delegates, entry.name) do
        nil ->
          [
            diagnostic(
              root,
              path,
              1,
              @ui_facade_rule,
              "catalog export #{entry.name}/1 has no matching public facade delegate"
            )
          ]

        %{target: target, function: function}
        when target == entry.module and function == entry.function ->
          []

        %{target: target, function: function} ->
          [
            diagnostic(
              root,
              path,
              1,
              @ui_facade_rule,
              "catalog export #{entry.name}/1 points to #{entry.module}.#{entry.function}/1, " <>
                "but the facade delegates to #{target}.#{function}/1"
            )
          ]
      end
    end)
  end

  defp implementation_contract_diagnostics(root, entries) do
    catalog_by_implementation =
      entries
      |> Enum.filter(&valid_catalog_entry?/1)
      |> Map.new(fn entry -> {{entry.module, entry.function}, entry} end)

    root
    |> ui_implementation_files()
    |> Enum.flat_map(fn path ->
      with {:ok, ast} <- parse_elixir(path),
           module when is_binary(module) <- defined_module(ast) do
        ast
        |> annotated_component_functions()
        |> Enum.flat_map(fn contract ->
          case Map.get(catalog_by_implementation, {module, contract.function}) do
            nil ->
              [
                diagnostic(
                  root,
                  path,
                  contract.line,
                  @ui_facade_rule,
                  "public implementation #{module}.#{contract.function}/1 has no facade catalog export"
                )
              ]

            entry ->
              attribute_findings =
                if MapSet.new(entry.attrs) == MapSet.new(contract.attrs) do
                  []
                else
                  [
                    diagnostic(
                      root,
                      path,
                      contract.line,
                      @ui_facade_rule,
                      "facade catalog attrs for #{entry.name} do not match implementation attrs " <>
                        "#{inspect(Enum.sort(contract.attrs))}"
                    )
                  ]
                end

              slot_findings =
                if MapSet.new(entry.slots) == MapSet.new(contract.slots) do
                  []
                else
                  [
                    diagnostic(
                      root,
                      path,
                      contract.line,
                      @ui_facade_rule,
                      "facade catalog slots for #{entry.name} do not match implementation slots " <>
                        "#{inspect(Enum.sort(contract.slots))}"
                    )
                  ]
                end

              attribute_findings ++ slot_findings
          end
        end)
      else
        nil -> []
        {:error, reason} -> [diagnostic(root, path, 1, @ui_facade_rule, reason)]
      end
    end)
  end

  defp check_showcase_and_test_parity(root) do
    case facade_catalog(root) do
      {:error, path, message} ->
        [diagnostic(root, path, 1, @ui_parity_rule, message)]

      {:ok, facade_path, _ast, entries} ->
        valid_entries = Enum.filter(entries, &valid_catalog_entry?/1)
        expected_showcases = MapSet.new(valid_entries, &normalize_identifier(&1.showcase_id))
        expected_tests = MapSet.new(valid_entries, &normalize_identifier(&1.a11y_test_id))
        {showcase_path, actual_showcases, showcase_findings} = showcase_ids(root)
        {actual_tests, test_paths} = accessibility_test_ids(root)

        showcase_parity =
          set_parity_diagnostics(
            root,
            showcase_path || facade_path,
            @ui_parity_rule,
            "showcase",
            expected_showcases,
            actual_showcases
          )

        test_path = List.first(test_paths) || facade_path

        test_parity =
          set_parity_diagnostics(
            root,
            test_path,
            @ui_parity_rule,
            "accessibility test",
            expected_tests,
            actual_tests
          )

        showcase_findings ++ showcase_parity ++ test_parity
    end
  end

  defp check_page_state_coverage(root) do
    allowed_states = MapSet.new([:empty | @page_statuses])

    root
    |> Path.join("lib/**/*_page.ex")
    |> Path.wildcard()
    |> Enum.flat_map(fn path ->
      with {:ok, ast} <- parse_elixir(path) do
        states = module_attribute_literal(ast, :states)
        test_path = page_test_path(root, path)
        covered_states = test_module_attribute(test_path, :covered_states)

        cond do
          not atom_list?(states) or states == [] ->
            [
              diagnostic(
                root,
                path,
                1,
                @ui_page_state_rule,
                "page must declare a non-empty literal @states list"
              )
            ]

          not MapSet.subset?(MapSet.new(states), allowed_states) ->
            [
              diagnostic(
                root,
                path,
                1,
                @ui_page_state_rule,
                "page @states contains variants outside the standard page-state vocabulary"
              )
            ]

          :ready not in states ->
            [
              diagnostic(
                root,
                path,
                1,
                @ui_page_state_rule,
                "page @states must include :ready"
              )
            ]

          not atom_list?(covered_states) ->
            [
              diagnostic(
                root,
                test_path,
                1,
                @ui_page_state_rule,
                "page render test must declare literal @covered_states"
              )
            ]

          MapSet.new(states) != MapSet.new(covered_states) ->
            [
              diagnostic(
                root,
                test_path,
                1,
                @ui_page_state_rule,
                "page render coverage #{inspect(Enum.sort(covered_states))} does not match " <>
                  "declared states #{inspect(Enum.sort(states))}"
              )
            ]

          true ->
            []
        end
      else
        {:error, reason} -> [diagnostic(root, path, 1, @ui_page_state_rule, reason)]
      end
    end)
  end

  defp live_view_files(root) do
    root
    |> Path.join("lib/**/*_live.ex")
    |> Path.wildcard()
    |> Enum.sort()
  end

  defp heex_page_components(path, source, start_line) do
    options = [
      file: path,
      line: start_line,
      tag_handler: Phoenix.LiveView.HTMLEngine,
      skip_macro_components: true
    ]

    case Phoenix.LiveView.TagEngine.Parser.parse(source, options) do
      {:ok, parser} -> page_component_tags(parser.nodes)
      {:error, _line, _column, _reason} -> []
    end
  end

  defp page_component_tags(nodes) do
    Enum.flat_map(nodes, fn
      {:block, :remote_component, name, _attrs, children, meta, _close_meta} ->
        own = if String.contains?(name, "Page."), do: [{name, meta[:line] || 1}], else: []
        own ++ page_component_tags(children)

      {:self_close, :remote_component, name, _attrs, meta} ->
        if String.contains?(name, "Page."), do: [{name, meta[:line] || 1}], else: []

      {:eex_block, _expression, clauses, _meta} ->
        Enum.flat_map(clauses, fn {children, _clause, _meta} -> page_component_tags(children) end)

      _node ->
        []
    end)
  end

  defp discover_tier_modules(root) do
    root
    |> elixir_files()
    |> Enum.reduce(%{}, fn path, modules ->
      source_tier = tier(path)

      with tier when tier in [:component, :composite, :page] <- source_tier,
           {:ok, ast} <- parse_elixir(path),
           module when is_binary(module) <- defined_module(ast) do
        Map.put(modules, module, %{module: module, path: path, tier: source_tier, ast: ast})
      else
        _ -> modules
      end
    end)
  end

  defp defined_module(ast) do
    {_ast, module} =
      Macro.prewalk(ast, nil, fn
        {:defmodule, _meta, [{:__aliases__, _alias_meta, parts} | _rest]} = node, nil ->
          {node, Enum.join(parts, ".")}

        node, module ->
          {node, module}
      end)

    module
  end

  defp placement_diagnostics(root, module, path, source_tier) do
    expected_segment =
      case source_tier do
        :component -> "Components"
        :composite -> "Composites"
        :page -> "Pages"
      end

    if expected_segment in String.split(module, ".") do
      []
    else
      [
        diagnostic(
          root,
          path,
          1,
          @ui_tier_rule,
          "module #{module} is in the #{source_tier} filesystem tier but not the #{expected_segment} namespace"
        )
      ]
    end
  end

  defp reference_diagnostics(root, module, path, source_tier, ast, modules) do
    ast
    |> module_references()
    |> Enum.reject(fn {_line, reference} -> reference == module end)
    |> Enum.flat_map(fn {line, reference} ->
      target_tier = modules[reference] && modules[reference].tier

      cond do
        source_tier in [:composite, :page] and implementation_module?(reference) ->
          [
            diagnostic(
              root,
              path,
              line,
              @ui_tier_rule,
              "#{source_tier} module #{module} imports implementation module #{reference}; " <>
                "depend on the public design-system facade"
            )
          ]

        source_tier in [:component, :composite] and
            forbidden_application_reference?(reference) ->
          [
            diagnostic(
              root,
              path,
              line,
              @ui_tier_rule,
              "#{source_tier} module #{module} imports forbidden application/runtime module " <>
                reference
            )
          ]

        target_tier && forbidden_tier_edge?(source_tier, target_tier) ->
          [
            diagnostic(
              root,
              path,
              line,
              @ui_tier_rule,
              "#{source_tier} module #{module} depends upward or sideways on " <>
                "#{target_tier} module #{reference}"
            )
          ]

        true ->
          []
      end
    end)
  end

  defp heex_tier_diagnostics(root, module, path, source_tier) do
    if source_tier in [:composite, :page] do
      path
      |> file_heex_sources()
      |> Enum.flat_map(fn {_path, source, start_line} ->
        case parse_heex(path, source, start_line) do
          {:ok, nodes} ->
            nodes
            |> heex_component_tags()
            |> Enum.filter(fn %{kind: kind, name: name} ->
              kind == :remote_component and implementation_component_name?(name)
            end)
            |> Enum.map(fn component ->
              diagnostic(
                root,
                path,
                component.line,
                @ui_tier_rule,
                "#{source_tier} module #{module} calls implementation component " <>
                  "<#{component.name}>; call the public design-system facade"
              )
            end)

          {:error, line, column, reason} ->
            [
              diagnostic(
                root,
                path,
                line,
                @ui_tier_rule,
                "cannot parse HEEx at column #{column}: #{reason}"
              )
            ]
        end
      end)
    else
      []
    end
  end

  defp composite_cycle_diagnostics(root, modules) do
    with {:ok, _path, _ast, entries} <- facade_catalog(root) do
      composite_exports =
        entries
        |> Enum.filter(&(valid_catalog_entry?(&1) and &1.tier == :composite))
        |> Map.new(&{Atom.to_string(&1.name), &1.module})

      edges =
        modules
        |> Map.values()
        |> Enum.filter(&(&1.tier == :composite))
        |> Enum.flat_map(fn %{module: module, path: path} ->
          path
          |> file_heex_sources()
          |> Enum.flat_map(fn {_path, source, start_line} ->
            case parse_heex(path, source, start_line) do
              {:ok, nodes} ->
                nodes
                |> heex_component_tags()
                |> Enum.flat_map(fn component ->
                  function_name = component.name |> String.split(".") |> List.last()

                  case Map.get(composite_exports, function_name) do
                    nil -> []
                    ^module -> []
                    target -> [{module, target, path, component.line}]
                  end
                end)

              {:error, _line, _column, _reason} ->
                []
            end
          end)
        end)

      graph =
        Enum.reduce(edges, %{}, fn {source, target, _path, _line}, graph ->
          Map.update(graph, source, MapSet.new([target]), &MapSet.put(&1, target))
        end)

      edges
      |> Enum.filter(fn {source, target, _path, _line} ->
        dependency_path?(graph, target, source, MapSet.new())
      end)
      |> Enum.map(fn {source, target, path, line} ->
        diagnostic(
          root,
          path,
          line,
          @ui_tier_rule,
          "composite dependency #{source} -> #{target} participates in a cycle"
        )
      end)
    else
      _ -> []
    end
  end

  defp dependency_path?(_graph, source, source, _visited), do: true

  defp dependency_path?(graph, source, target, visited) do
    if MapSet.member?(visited, source) do
      false
    else
      graph
      |> Map.get(source, MapSet.new())
      |> Enum.any?(&dependency_path?(graph, &1, target, MapSet.put(visited, source)))
    end
  end

  defp parse_heex(path, source, start_line) do
    options = [
      file: path,
      line: start_line,
      tag_handler: Phoenix.LiveView.HTMLEngine,
      skip_macro_components: true
    ]

    case Phoenix.LiveView.TagEngine.Parser.parse(source, options) do
      {:ok, parser} -> {:ok, parser.nodes}
      {:error, line, column, reason} -> {:error, line, column, reason}
    end
  end

  defp heex_attributes(nodes) do
    Enum.flat_map(nodes, fn
      {:block, _kind, _name, attributes, children, _meta, _close_meta} ->
        normalize_heex_attributes(attributes) ++ heex_attributes(children)

      {:self_close, _kind, _name, attributes, _meta} ->
        normalize_heex_attributes(attributes)

      {:eex_block, _expression, clauses, _meta} ->
        Enum.flat_map(clauses, fn {children, _clause, _meta} -> heex_attributes(children) end)

      _node ->
        []
    end)
  end

  defp normalize_heex_attributes(attributes) do
    Enum.flat_map(attributes, fn
      {name, {kind, value, _value_meta}, meta}
      when is_binary(name) and kind in [:expr, :string] ->
        [%{name: name, kind: kind, value: value, line: meta[:line] || 1}]

      _attribute ->
        []
    end)
  end

  defp heex_component_tags(nodes) do
    Enum.flat_map(nodes, fn
      {:block, kind, name, _attributes, children, meta, _close_meta}
      when kind in [:local_component, :remote_component] ->
        [%{kind: kind, name: name, line: meta[:line] || 1} | heex_component_tags(children)]

      {:self_close, kind, name, _attributes, meta}
      when kind in [:local_component, :remote_component] ->
        [%{kind: kind, name: name, line: meta[:line] || 1}]

      {:block, _kind, _name, _attributes, children, _meta, _close_meta} ->
        heex_component_tags(children)

      {:eex_block, _expression, clauses, _meta} ->
        Enum.flat_map(clauses, fn {children, _clause, _meta} -> heex_component_tags(children) end)

      _node ->
        []
    end)
  end

  defp declared_interaction_expression?(%{kind: :string}), do: false

  defp declared_interaction_expression?(%{kind: :expr, value: source}) do
    case Code.string_to_quoted(source) do
      {:ok, ast} -> expression_has_runtime_value?(ast)
      {:error, _reason} -> false
    end
  end

  defp expression_has_runtime_value?(ast) do
    {_ast, found?} =
      Macro.prewalk(ast, false, fn
        {:@, _meta, _arguments} = node, _found? ->
          {node, true}

        {name, _meta, context} = node, found?
        when is_atom(name) and (is_atom(context) or is_nil(context)) and
               name not in [nil, true, false] ->
          {node, true or found?}

        node, found? ->
          {node, found?}
      end)

    found?
  end

  defp component_attribute_declarations(ast) do
    {_ast, declarations} =
      Macro.prewalk(ast, [], fn
        {:attr, meta, [name, type | options]} = node, declarations when is_atom(name) ->
          options = if length(options) == 1 and is_list(hd(options)), do: hd(options), else: []

          declaration = %{
            name: name,
            type: type,
            options: options,
            line: meta[:line] || 1
          }

          {node, [declaration | declarations]}

        node, declarations ->
          {node, declarations}
      end)

    Enum.reverse(declarations)
  end

  defp bounded_attribute?(%{options: options}) do
    case Keyword.get(options, :values) do
      values when is_list(values) and values != [] -> Enum.all?(values, &literal_scalar?/1)
      _other -> false
    end
  end

  defp interaction_property_name?(name) when is_atom(name) do
    name in @interaction_property_names or String.starts_with?(Atom.to_string(name), "on_")
  end

  defp bounded_interaction_attribute?(%{type: type} = declaration) do
    type in [:atom, :string, :integer, :boolean] and bounded_attribute?(declaration)
  end

  defp literal_scalar?(value), do: is_atom(value) or is_binary(value) or is_number(value)

  defp module_references(ast) do
    {_ast, references} =
      Macro.prewalk(ast, MapSet.new(), fn
        {:__aliases__, meta, parts} = node, references ->
          {node, MapSet.put(references, {meta[:line] || 1, Enum.join(parts, ".")})}

        node, references ->
          {node, references}
      end)

    Enum.sort(references)
  end

  defp implementation_module?(reference) do
    String.contains?(reference, [
      ".DesignSystem.Components.",
      ".DesignSystem.Composites.",
      "CoreComponents",
      "OrganizationComponents",
      "ProjectComponents",
      "RepositoryComponents",
      "ResourceComponents",
      ".Layouts"
    ])
  end

  defp implementation_component_name?(name) do
    String.contains?(name, [".Components.", ".Composites."])
  end

  defp forbidden_application_reference?(reference) do
    String.starts_with?(reference, "HephaestusWeb.") or
      String.starts_with?(reference, "HephaestusWebWeb.Generated.") or
      String.contains?(reference, [".Generated.", ".Rpc.", ".RPC."]) or
      String.ends_with?(reference, ["Client", "Stub", "State", "Router", "Routes", "Endpoint"]) or
      String.contains?(reference, [".State.", ".Pages."]) or
      String.starts_with?(reference, "Phoenix.LiveView") or
      Regex.match?(~r/^HephaestusWebWeb\..*Live$/, reference) or
      reference == "HephaestusWebWeb"
  end

  defp external_ui_module?(reference) do
    String.starts_with?(reference, [
      "Heroicons",
      "DaisyUI",
      "Petal",
      "Surface",
      "Phoenix.HTML"
    ]) or
      reference in [
        "HephaestusWebWeb.CoreComponents",
        "HephaestusWebWeb.OrganizationComponents",
        "HephaestusWebWeb.ProjectComponents",
        "HephaestusWebWeb.RepositoryComponents",
        "HephaestusWebWeb.ResourceComponents"
      ]
  end

  defp javascript_import_diagnostics(root, path) do
    case File.read(path) do
      {:ok, source} ->
        allowed = [
          "phoenix",
          "phoenix_html",
          "phoenix_live_view",
          "phoenix-colocated/hephaestus_web"
        ]

        source
        |> String.split("\n")
        |> Enum.with_index(1)
        |> Enum.flat_map(fn {line_source, line} ->
          case Regex.run(
                 ~r/^\s*import(?:\s+.+?\s+from\s+|\s*)["']([^"']+)["']/,
                 line_source,
                 capture: :all_but_first
               ) do
            [specifier] ->
              if specifier in allowed or
                   String.starts_with?(specifier, ["./design_system/", "../design_system/"]) do
                []
              else
                [
                  diagnostic(
                    root,
                    path,
                    line,
                    @ui_external_rule,
                    "JavaScript imports external UI implementation #{inspect(specifier)} outside assets/js/design_system"
                  )
                ]
              end

            _other ->
              []
          end
        end)

      {:error, reason} ->
        [
          diagnostic(
            root,
            path,
            1,
            @ui_external_rule,
            "cannot inspect JavaScript source: #{format_error(reason)}"
          )
        ]
    end
  end

  defp javascript_code_only(source) do
    source
    |> String.to_charlist()
    |> scrub_javascript(:code, [])
    |> Enum.reverse()
    |> List.to_string()
  end

  defp scrub_javascript([], _state, output), do: output

  defp scrub_javascript([?/, ?/ | rest], :code, output),
    do: scrub_javascript(rest, {:line_comment, :code}, [32, 32 | output])

  defp scrub_javascript([?/, ?/ | rest], {:template_expression, _depth} = state, output),
    do: scrub_javascript(rest, {:line_comment, state}, [32, 32 | output])

  defp scrub_javascript([?/, ?* | rest], :code, output),
    do: scrub_javascript(rest, {:block_comment, :code}, [32, 32 | output])

  defp scrub_javascript([?/, ?* | rest], {:template_expression, _depth} = state, output),
    do: scrub_javascript(rest, {:block_comment, state}, [32, 32 | output])

  defp scrub_javascript([quote | rest], :code, output) when quote in [?", ?'],
    do: scrub_javascript(rest, {:string, quote, :code, false}, [32 | output])

  defp scrub_javascript(
         [quote | rest],
         {:template_expression, _depth} = state,
         output
       )
       when quote in [?", ?'],
       do: scrub_javascript(rest, {:string, quote, state, false}, [32 | output])

  defp scrub_javascript([?` | rest], :code, output),
    do: scrub_javascript(rest, :template, [32 | output])

  defp scrub_javascript([?{ | rest], {:template_expression, depth}, output),
    do: scrub_javascript(rest, {:template_expression, depth + 1}, [?{ | output])

  defp scrub_javascript([?} | rest], {:template_expression, 1}, output),
    do: scrub_javascript(rest, :template, [32 | output])

  defp scrub_javascript([?} | rest], {:template_expression, depth}, output),
    do: scrub_javascript(rest, {:template_expression, depth - 1}, [?} | output])

  defp scrub_javascript([character | rest], state, output)
       when state == :code or (is_tuple(state) and elem(state, 0) == :template_expression),
       do: scrub_javascript(rest, state, [character | output])

  defp scrub_javascript([?\n | rest], {:line_comment, return_state}, output),
    do: scrub_javascript(rest, return_state, [?\n | output])

  defp scrub_javascript([_character | rest], {:line_comment, _return_state} = state, output),
    do: scrub_javascript(rest, state, [32 | output])

  defp scrub_javascript([?*, ?/ | rest], {:block_comment, return_state}, output),
    do: scrub_javascript(rest, return_state, [32, 32 | output])

  defp scrub_javascript([?\n | rest], {:block_comment, _return_state} = state, output),
    do: scrub_javascript(rest, state, [?\n | output])

  defp scrub_javascript([_character | rest], {:block_comment, _return_state} = state, output),
    do: scrub_javascript(rest, state, [32 | output])

  defp scrub_javascript([_character | rest], {:string, quote, return_state, true}, output),
    do: scrub_javascript(rest, {:string, quote, return_state, false}, [32 | output])

  defp scrub_javascript([?\\ | rest], {:string, quote, return_state, false}, output),
    do: scrub_javascript(rest, {:string, quote, return_state, true}, [32 | output])

  defp scrub_javascript([quote | rest], {:string, quote, return_state, false}, output),
    do: scrub_javascript(rest, return_state, [32 | output])

  defp scrub_javascript(
         [?\n | rest],
         {:string, _quote, _return_state, _escaped?} = state,
         output
       ),
       do: scrub_javascript(rest, state, [?\n | output])

  defp scrub_javascript(
         [_character | rest],
         {:string, _quote, _return_state, _escaped?} = state,
         output
       ),
       do: scrub_javascript(rest, state, [32 | output])

  defp scrub_javascript([?\\, _escaped | rest], :template, output),
    do: scrub_javascript(rest, :template, [32, 32 | output])

  defp scrub_javascript([?$, ?{ | rest], :template, output),
    do: scrub_javascript(rest, {:template_expression, 1}, [32, 32 | output])

  defp scrub_javascript([?` | rest], :template, output),
    do: scrub_javascript(rest, :code, [32 | output])

  defp scrub_javascript([?\n | rest], :template, output),
    do: scrub_javascript(rest, :template, [?\n | output])

  defp scrub_javascript([_character | rest], :template, output),
    do: scrub_javascript(rest, :template, [32 | output])

  defp facade_catalog(root) do
    case root |> Path.join("lib/**/design_system.ex") |> Path.wildcard() |> Enum.sort() do
      [] ->
        {:error, Path.join(root, "lib"), "public design-system facade is missing"}

      [path | _rest] ->
        with {:ok, ast} <- parse_elixir(path),
             {:ok, body} <- zero_arity_function_body(ast, :catalog),
             {:ok, entries} <- literal_value(body, alias_map(ast)),
             true <- is_list(entries) do
          {:ok, path, ast, entries}
        else
          {:error, reason} -> {:error, path, "cannot inspect facade catalog: #{reason}"}
          false -> {:error, path, "facade catalog/0 must return a literal list"}
        end
    end
  end

  defp zero_arity_function_body(ast, function) do
    {_ast, body} =
      Macro.prewalk(ast, nil, fn
        {:def, _meta, [{^function, _function_meta, arguments}, [do: body]]} = node, nil
        when arguments in [[], nil] ->
          {node, body}

        node, body ->
          {node, body}
      end)

    if is_nil(body), do: {:error, "missing #{function}/0"}, else: {:ok, body}
  end

  defp literal_value(value, _aliases)
       when is_atom(value) or is_binary(value) or is_number(value),
       do: {:ok, value}

  defp literal_value(values, aliases) when is_list(values) do
    Enum.reduce_while(values, {:ok, []}, fn value, {:ok, decoded} ->
      case literal_value(value, aliases) do
        {:ok, decoded_value} -> {:cont, {:ok, [decoded_value | decoded]}}
        {:error, reason} -> {:halt, {:error, reason}}
      end
    end)
    |> case do
      {:ok, decoded} -> {:ok, Enum.reverse(decoded)}
      error -> error
    end
  end

  defp literal_value({:%{}, _meta, pairs}, aliases) do
    Enum.reduce_while(pairs, {:ok, %{}}, fn {key_ast, value_ast}, {:ok, decoded} ->
      with {:ok, key} <- literal_value(key_ast, aliases),
           {:ok, value} <- literal_value(value_ast, aliases) do
        {:cont, {:ok, Map.put(decoded, key, value)}}
      else
        {:error, reason} -> {:halt, {:error, reason}}
      end
    end)
  end

  defp literal_value({:__aliases__, _meta, parts}, aliases) do
    reference = Enum.join(parts, ".")
    {:ok, Map.get(aliases, reference, reference)}
  end

  defp literal_value({left, right}, aliases) do
    with {:ok, decoded_left} <- literal_value(left, aliases),
         {:ok, decoded_right} <- literal_value(right, aliases) do
      {:ok, {decoded_left, decoded_right}}
    end
  end

  defp literal_value(other, _aliases),
    do: {:error, "non-literal expression #{Macro.to_string(other)}"}

  defp alias_map(ast) do
    {_ast, aliases} =
      Macro.prewalk(ast, %{}, fn
        {:alias, _meta, [{:__aliases__, _alias_meta, parts}]} = node, aliases ->
          full = Enum.join(parts, ".")
          {node, Map.put(aliases, List.last(parts) |> Atom.to_string(), full)}

        {:alias, _meta, [{:__aliases__, _alias_meta, parts}, options]} = node, aliases
        when is_list(options) ->
          full = Enum.join(parts, ".")

          short =
            case Keyword.get(options, :as) do
              {:__aliases__, _as_meta, as_parts} -> List.last(as_parts) |> Atom.to_string()
              _other -> List.last(parts) |> Atom.to_string()
            end

          {node, Map.put(aliases, short, full)}

        node, aliases ->
          {node, aliases}
      end)

    aliases
  end

  defp resolve_module({:__aliases__, _meta, parts}, aliases) do
    reference = Enum.join(parts, ".")
    Map.get(aliases, reference, reference)
  end

  defp resolve_module(module, _aliases) when is_atom(module), do: Atom.to_string(module)
  defp resolve_module(module, _aliases), do: inspect(module)

  defp facade_delegates(ast, aliases) do
    {_ast, delegates} =
      Macro.prewalk(ast, %{}, fn
        {:defdelegate, _meta, [{name, _function_meta, arguments}, options]} = node, delegates
        when is_atom(name) and is_list(arguments) and length(arguments) == 1 and
               is_list(options) ->
          target = options |> Keyword.fetch!(:to) |> resolve_module(aliases)
          function = Keyword.get(options, :as, name)
          {node, Map.put(delegates, name, %{target: target, function: function})}

        node, delegates ->
          {node, delegates}
      end)

    delegates
  end

  defp annotated_component_functions(ast) do
    ast
    |> module_expressions()
    |> Enum.reduce({[], [], []}, fn expression, {contracts, attrs, slots} ->
      case expression do
        {:attr, _meta, [name | _arguments]} when is_atom(name) ->
          {contracts, [name | attrs], slots}

        {:slot, _meta, [name | _arguments]} when is_atom(name) ->
          {contracts, attrs, [name | slots]}

        {:def, meta, [{function, _function_meta, arguments} | _rest]}
        when is_atom(function) and is_list(arguments) and length(arguments) == 1 and
               (attrs != [] or slots != []) ->
          contract = %{
            function: function,
            attrs: Enum.reverse(attrs),
            slots: Enum.reverse(slots),
            line: meta[:line] || 1
          }

          {[contract | contracts], [], []}

        _other ->
          {contracts, attrs, slots}
      end
    end)
    |> elem(0)
    |> Enum.reverse()
  end

  defp module_expressions(ast) do
    {_ast, body} =
      Macro.prewalk(ast, nil, fn
        {:defmodule, _meta, [_module, [do: body]]} = node, nil -> {node, body}
        node, body -> {node, body}
      end)

    case body do
      {:__block__, _meta, expressions} -> expressions
      nil -> []
      expression -> [expression]
    end
  end

  defp valid_catalog_entry?(entry) do
    is_map(entry) and Enum.all?(@catalog_keys, &Map.has_key?(entry, &1)) and
      is_atom(entry.name) and entry.tier in [:component, :composite] and
      is_binary(entry.module) and is_atom(entry.function) and atom_list?(entry.attrs) and
      atom_list?(entry.slots) and identifier?(entry.showcase_id) and
      identifier?(entry.a11y_test_id)
  end

  defp atom_list?(value), do: is_list(value) and Enum.all?(value, &is_atom/1)

  defp identifier?(value), do: is_atom(value) or (is_binary(value) and value != "")
  defp normalize_identifier(value) when is_atom(value), do: Atom.to_string(value)
  defp normalize_identifier(value), do: value

  defp showcase_ids(root) do
    case root
         |> Path.join("lib/**/design_system/showcase.ex")
         |> Path.wildcard()
         |> Enum.sort() do
      [] ->
        {nil, MapSet.new(),
         [
           diagnostic(
             root,
             Path.join(root, "lib"),
             1,
             @ui_parity_rule,
             "design-system showcase module is missing"
           )
         ]}

      [path | _rest] ->
        with {:ok, ast} <- parse_elixir(path),
             {:ok, body} <- zero_arity_function_body(ast, :examples),
             {:ok, examples} <- literal_value(body, alias_map(ast)) do
          {path, normalize_example_ids(examples), []}
        else
          {:error, reason} ->
            {path, MapSet.new(),
             [diagnostic(root, path, 1, @ui_parity_rule, "cannot inspect showcase: #{reason}")]}
        end
    end
  end

  defp normalize_example_ids(examples) when is_list(examples) do
    examples
    |> Enum.flat_map(fn
      value when is_binary(value) or is_atom(value) ->
        [normalize_identifier(value)]

      %{showcase_id: value} when is_binary(value) or is_atom(value) ->
        [normalize_identifier(value)]

      %{id: value} when is_binary(value) or is_atom(value) ->
        [normalize_identifier(value)]

      _other ->
        []
    end)
    |> MapSet.new()
  end

  defp normalize_example_ids(examples) when is_map(examples) do
    examples
    |> Map.keys()
    |> Enum.map(&normalize_identifier/1)
    |> Enum.filter(&is_binary/1)
    |> MapSet.new()
  end

  defp normalize_example_ids(_examples), do: MapSet.new()

  defp accessibility_test_ids(root) do
    root
    |> Path.join("test/**/*_test.exs")
    |> Path.wildcard()
    |> Enum.reject(&String.starts_with?(&1, Path.join(root, "test/fixtures/")))
    |> Enum.reduce({MapSet.new(), []}, fn path, {ids, paths} ->
      case parse_elixir(path) do
        {:ok, ast} ->
          case module_attribute_literal(ast, :a11y_test_ids) do
            values when is_list(values) ->
              decoded =
                values
                |> Enum.map(&normalize_identifier/1)
                |> Enum.filter(&is_binary/1)
                |> MapSet.new()

              {MapSet.union(ids, decoded), [path | paths]}

            _other ->
              {ids, paths}
          end

        {:error, _reason} ->
          {ids, paths}
      end
    end)
  end

  defp set_parity_diagnostics(root, path, rule, label, expected, actual) do
    missing = MapSet.difference(expected, actual) |> Enum.sort()
    extra = MapSet.difference(actual, expected) |> Enum.sort()

    missing_findings =
      Enum.map(missing, fn id ->
        diagnostic(root, path, 1, rule, "missing #{label} parity entry #{inspect(id)}")
      end)

    extra_findings =
      Enum.map(extra, fn id ->
        diagnostic(root, path, 1, rule, "undeclared #{label} parity entry #{inspect(id)}")
      end)

    missing_findings ++ extra_findings
  end

  defp module_attribute_literal(ast, name) do
    aliases = alias_map(ast)

    {_ast, value} =
      Macro.prewalk(ast, nil, fn
        {:@, _meta, [{^name, _name_meta, [value_ast]}]} = node, nil ->
          case literal_value(value_ast, aliases) do
            {:ok, decoded} -> {node, decoded}
            {:error, _reason} -> {node, :non_literal}
          end

        node, value ->
          {node, value}
      end)

    value
  end

  defp page_test_path(root, page_path) do
    basename = Path.basename(page_path, ".ex") <> "_test.exs"

    root
    |> Path.join("test/**/#{basename}")
    |> Path.wildcard()
    |> List.first()
    |> case do
      nil -> Path.join(root, "test/#{basename}")
      path -> path
    end
  end

  defp test_module_attribute(path, name) do
    case parse_elixir(path) do
      {:ok, ast} -> module_attribute_literal(ast, name)
      {:error, _reason} -> nil
    end
  end

  defp forbidden_tier_edge?(:component, target), do: target != :component
  defp forbidden_tier_edge?(:composite, target), do: target != :component
  defp forbidden_tier_edge?(:page, target), do: target == :page

  defp heex_sources(root) do
    template_sources =
      root
      |> Path.join("lib/**/*.html.heex")
      |> Path.wildcard()
      |> Enum.flat_map(fn path ->
        case File.read(path) do
          {:ok, source} -> [{path, source, 1}]
          {:error, _reason} -> []
        end
      end)

    sigil_sources = root |> elixir_files() |> Enum.flat_map(&file_heex_sources/1)

    Enum.sort_by(template_sources ++ sigil_sources, fn {path, _source, line} -> {path, line} end)
  end

  defp file_heex_sources(path) do
    with {:ok, ast} <- parse_elixir(path) do
      {_ast, sources} =
        Macro.prewalk(ast, [], fn
          {:sigil_H, meta, [{:<<>>, _binary_meta, [source]}, _modifiers]} = node, sources
          when is_binary(source) ->
            {node, [{path, source, meta[:line] || 1} | sources]}

          node, sources ->
            {node, sources}
        end)

      sources
    else
      _ -> []
    end
  end

  defp elixir_files(root),
    do: root |> Path.join("lib/**/*.{ex,exs}") |> Path.wildcard() |> Enum.sort()

  defp ui_elixir_files(root) do
    root
    |> elixir_files()
    |> Enum.filter(
      &(tier(&1) in [:component, :composite, :page] or design_system_facade_path?(&1))
    )
  end

  defp ui_implementation_files(root) do
    root
    |> elixir_files()
    |> Enum.filter(&(tier(&1) in [:component, :composite]))
  end

  defp parse_elixir(path) do
    with {:ok, source} <- File.read(path),
         {:ok, ast} <- Code.string_to_quoted(source, file: path) do
      {:ok, ast}
    else
      {:error, {_line, error, token}} ->
        {:error, "cannot parse source: #{error} #{inspect(token)}"}

      {:error, reason} ->
        {:error, "cannot inspect source: #{format_error(reason)}"}
    end
  end

  defp tier(path) do
    normalized = String.replace(path, "\\", "/")

    cond do
      String.contains?(normalized, "/design_system/components/") -> :component
      String.contains?(normalized, "/design_system/composites/") -> :composite
      String.contains?(normalized, "/pages/") -> :page
      true -> nil
    end
  end

  defp design_system_facade_path?(path) do
    normalized = String.replace(path, "\\", "/")
    String.ends_with?(normalized, "/design_system.ex")
  end

  defp design_system_rendering_path?(path) do
    normalized = String.replace(path, "\\", "/")

    Enum.any?(
      [
        "/design_system/components/",
        "/design_system/composites/",
        "/design_system/pages/"
      ],
      &String.contains?(normalized, &1)
    )
  end

  defp design_system_css_path?(path) do
    normalized = String.replace(path, "\\", "/")
    String.contains?(normalized, "/assets/css/design_system/")
  end

  defp design_system_javascript_path?(path) do
    normalized = String.replace(path, "\\", "/")
    String.contains?(normalized, "/assets/js/design_system/")
  end

  defp design_system_hook_path?(path) do
    normalized = String.replace(path, "\\", "/")
    String.contains?(normalized, "/assets/js/design_system/hooks/")
  end

  defp diagnostic(root, path, line, rule, message) do
    %Diagnostic{
      rule: rule,
      path: Path.relative_to(path, root),
      line: max(line, 1),
      message: message
    }
  end

  defp format_error(reason) when is_atom(reason), do: :file.format_error(reason) |> to_string()
  defp format_error(reason), do: inspect(reason)
end
