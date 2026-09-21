defmodule HephaestusWebWeb.SessionChatNewState do
  @moduledoc "State and effects for composing one repository-backed session chat."

  alias HephaestusWeb.RPC.Client
  alias HephaestusWeb.RPC.UUID
  alias HephaestusWeb.UIBrowser

  @stream_mode :none
  @statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  defstruct status: :initial,
            data: %{
              project_id: nil,
              project: nil,
              release_catalog: [],
              secret_imports: [],
              progress: %{},
              __model_rule_id: nil,
              __attempt_id: nil,
              __client: Client
            },
            form: %{
              "repository_name" => "session-chat",
              "default_branch" => "main",
              "instance_name" => "Session chat",
              "release_agent_id" => "",
              "model_import_id" => "",
              "acknowledge_repository_git_access" => "false",
              "setup_attempt_id" => ""
            },
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(project_id) do
    attempt_id = UUID.generate()

    %__MODULE__{
      data: %{
        project_id: project_id,
        project: nil,
        release_catalog: [],
        secret_imports: [],
        progress: %{},
        __model_rule_id: UUID.generate(),
        __attempt_id: attempt_id,
        __client: Client
      },
      form: %{
        "repository_name" => "session-chat",
        "default_branch" => "main",
        "instance_name" => "Session chat",
        "release_agent_id" => "",
        "model_import_id" => "",
        "acknowledge_repository_git_access" => "false",
        "setup_attempt_id" => attempt_id
      }
    }
  end

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode

  def reduce(state, :load), do: {%{state | status: :loading, error: nil}, [:load]}
  def reduce(state, :submitting), do: {%{state | status: :submitting, error: nil}, []}
  def reduce(state, {:form, form}), do: {%{state | form: form}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}
  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}

  def reduce(state, {:loaded, {:ok, project, release_catalog, secret_imports}}),
    do:
      {%{
         state
         | status: :ready,
           data: %{
             state.data
             | project: project,
               release_catalog: release_catalog,
               secret_imports: secret_imports
           }
       }, []}

  def reduce(state, {:loaded, {:error, reason}}), do: reduce(state, {:failed, reason})

  def reduce(state, {:created, {:ok, url, repository_id, instance_id}}),
    do:
      {%{state | status: :ready, error: nil},
       [
         {:launch, url},
         {:flash, :info,
          "Session chat created in repository #{repository_id} (instance #{instance_id})."}
       ]}

  def reduce(state, {:created, {:error, reason}}), do: reduce(state, {:failed, reason})

  def reduce(state, {:created, {:partial, progress, reason}}) do
    message = "Session setup stopped after creating " <> progress_summary(progress) <> "."

    {%{
       state
       | status: :error,
         error: present_error(reason),
         data: %{state.data | progress: progress}
     }, [{:flash, :error, message}]}
  end

  def reduce(state, {:failed, reason}),
    do:
      {%{state | status: :error, error: present_error(reason)},
       [{:flash, :error, present_error(reason)}]}

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "That project is not visible."},
       [{:navigate, "/organizations"}]}

  def present(state),
    do: %{
      state: presentation_status(state),
      project: state.data.project,
      project_id: state.data.project_id,
      release_catalog: state.data.release_catalog,
      secret_imports: state.data.secret_imports,
      progress: state.data.progress,
      attempt_id: Map.get(state.data, :__attempt_id),
      form: state.form,
      error: state.error
    }

  def execute(state, {:load, identity}) do
    client = rpc_client(state)

    with {:ok, project} <- client.get_project(identity, state.data.project_id),
         {:ok, catalog} <- client.list_importable_release_agents(identity, state.data.project_id),
         {:ok, authority} <- client.list_project_secret_authority(identity, state.data.project_id) do
      {:loaded,
       {:ok, project, Enum.filter(catalog, &compatible_release?/1), authority["imports"] || []}}
    else
      {:error, reason} -> {:loaded, {:error, reason}}
    end
  end

  def execute(state, {:create, identity, attributes}) do
    client = rpc_client(state)

    model_rule_id =
      Map.get(state.data.progress, "model_rule_id") || Map.fetch!(state.data, :__model_rule_id)

    with :ok <- acknowledge_git?(attributes),
         :ok <- exact_main_branch?(attributes),
         :ok <- valid_attempt_id?(state, attributes),
         {:ok, release_agent} <-
           find_release_agent(state.data.release_catalog, attributes["release_agent_id"]),
         {:ok, model_import} <-
           find_import(state.data.secret_imports, attributes["model_import_id"]),
         :ok <- compatible_model_import?(model_import),
         {:ok, descriptor} <- session_ui_descriptor(client, identity, release_agent),
         :ok <-
           matching_attempt?(
             state.data.progress,
             attributes,
             release_agent,
             model_import,
             descriptor,
             model_rule_id
           ) do
      progress = state.data.progress

      case progress["repository_id"] do
        repository_id when is_binary(repository_id) ->
          compose_after_repository(
            identity,
            state,
            attributes,
            release_agent,
            model_import,
            descriptor,
            %{"repository_id" => repository_id},
            progress
          )

        _missing ->
          case client.create_repository(
                 identity,
                 state.data.project_id,
                 attributes["repository_name"] || "",
                 "main",
                 false,
                 true,
                 rpc_options(state, :create_repository)
               ) do
            {:ok, repository} ->
              compose_after_repository(
                identity,
                state,
                attributes,
                release_agent,
                model_import,
                descriptor,
                repository,
                Map.merge(progress, %{
                  "repository_id" => repository["repository_id"],
                  "model_rule_id" => model_rule_id,
                  "setup_config" =>
                    setup_config(
                      attributes,
                      release_agent,
                      model_import,
                      descriptor,
                      model_rule_id
                    )
                })
              )

            {:error, reason} ->
              {:created, {:error, reason}}
          end
      end
    else
      {:error, reason} -> {:created, {:error, reason}}
    end
  end

  defp compose_after_repository(
         identity,
         state,
         attributes,
         release_agent,
         model_import,
         descriptor,
         repository,
         progress
       ) do
    model_rule_id = Map.fetch!(progress, "model_rule_id")

    imported_result =
      case progress["instance_id"] do
        instance_id when is_binary(instance_id) ->
          {:ok,
           %{
             "instance_id" => instance_id,
             "revision_id" => progress["revision_id"]
           }, progress}

        _missing ->
          case rpc_client(state).import_agent(
                 identity,
                 state.data.project_id,
                 release_agent["id"],
                 attributes["instance_name"] || "Session chat",
                 %{"model_rule_id" => model_rule_id},
                 selected_policy(release_agent),
                 rpc_options(state, :import_agent)
               ) do
            {:ok, imported} ->
              {:ok, imported,
               Map.merge(progress, %{
                 "instance_id" => imported["instance_id"],
                 "revision_id" => imported["revision_id"]
               })}

            {:error, reason} ->
              {:error, reason, progress}
          end
      end

    case imported_result do
      {:ok, imported, progress} ->
        capabilities_result =
          case progress["capability_revision_id"] do
            capability_revision_id when is_binary(capability_revision_id) ->
              {:ok, %{"instance_revision_id" => capability_revision_id}, progress}

            _missing ->
              case revise_session_capability(
                     rpc_client(state),
                     identity,
                     state,
                     imported,
                     release_agent,
                     repository
                   ) do
                {:ok, capabilities} ->
                  {:ok, capabilities,
                   Map.put(
                     progress,
                     "capability_revision_id",
                     capabilities["instance_revision_id"]
                   )}

                {:error, reason} ->
                  {:error, reason, progress}
              end
          end

        case capabilities_result do
          {:ok, capabilities, progress} ->
            create_attachment_and_continue(
              identity,
              state,
              release_agent,
              model_import,
              descriptor,
              repository,
              imported,
              capabilities,
              progress
            )

          {:error, reason, progress} ->
            {:created, {:partial, progress, reason}}
        end

      {:error, reason, progress} ->
        {:created, {:partial, progress, reason}}
    end
  end

  defp create_attachment_and_continue(
         identity,
         state,
         release_agent,
         model_import,
         descriptor,
         repository,
         imported,
         capabilities,
         progress
       ) do
    attachment_result =
      case progress["attachment_id"] do
        attachment_id when is_binary(attachment_id) ->
          {:ok, %{"attachment_id" => attachment_id}, progress}

        _missing ->
          case rpc_client(state).create_attachment(
                 identity,
                 imported["instance_id"],
                 repository["repository_id"],
                 %{"type" => "exact", "value" => "refs/heads/main"},
                 "push",
                 rpc_options(state, :create_attachment)
               ) do
            {:ok, attachment} ->
              {:ok, attachment, Map.put(progress, "attachment_id", attachment["attachment_id"])}

            {:error, reason} ->
              {:error, reason, progress}
          end
      end

    case attachment_result do
      {:ok, attachment, progress} ->
        bind_and_install(
          identity,
          state,
          release_agent,
          model_import,
          descriptor,
          repository,
          imported,
          capabilities,
          attachment,
          progress
        )

      {:error, reason, progress} ->
        {:created, {:partial, progress, reason}}
    end
  end

  defp bind_and_install(
         identity,
         state,
         release_agent,
         model_import,
         descriptor,
         repository,
         imported,
         capabilities,
         attachment,
         progress
       ) do
    with {:ok, progress} <-
           ensure_secret_binding(
             identity,
             state,
             model_import,
             imported,
             capabilities,
             attachment,
             progress
           ),
         {:ok, installation, install_progress} <-
           ensure_ui_installation(
             identity,
             state,
             release_agent,
             descriptor,
             repository,
             progress
           ) do
      case handoff(rpc_client(state), identity, installation, descriptor["route_base"]) do
        {:ok, url} ->
          {:created, {:ok, url, repository["repository_id"], imported["instance_id"]}}

        {:error, reason} ->
          {:created, {:partial, install_progress, reason}}
      end
    else
      {:error, reason, progress} -> {:created, {:partial, progress, reason}}
    end
  end

  defp ensure_secret_binding(
         identity,
         state,
         model_import,
         imported,
         capabilities,
         attachment,
         progress
       ) do
    with {:ok, progress} <-
           ensure_secret_binding_revision(
             identity,
             state,
             model_import,
             imported,
             capabilities,
             attachment,
             progress
           ),
         {:ok, progress} <- ensure_brokered_https_rule(identity, state, progress) do
      {:ok, progress}
    else
      {:error, reason, progress} -> {:error, reason, progress}
    end
  end

  defp ensure_secret_binding_revision(
         identity,
         state,
         model_import,
         imported,
         capabilities,
         attachment,
         progress
       ) do
    if progress["secret_bound"] do
      if is_binary(progress["binding_id"]),
        do: {:ok, progress},
        else: {:error, :missing_binding_id, progress}
    else
      case rpc_client(state).bind_secret(
             identity,
             %{
               "instance_id" => imported["instance_id"],
               "expected_revision_id" => capabilities["instance_revision_id"],
               "import_id" => model_import["id"],
               "slot" => "model",
               "mode" => "brokered",
               "phases" => ["normal"],
               "attachment_ids" => [attachment["attachment_id"]],
               "destinations" => ["api.model.example"]
             },
             rpc_options(state, :bind_secret)
           ) do
        {:ok, %{"binding_id" => binding_id}} when is_binary(binding_id) ->
          {:ok, Map.merge(progress, %{"secret_bound" => true, "binding_id" => binding_id})}

        {:ok, _binding} ->
          {:error, :missing_binding_id, progress}

        {:error, reason} ->
          {:error, reason, progress}
      end
    end
  end

  defp ensure_brokered_https_rule(identity, state, progress) do
    if progress["brokered_rule_declared"] do
      {:ok, progress}
    else
      case rpc_client(state).declare_brokered_https_rule(
             identity,
             %{
               "binding_id" => progress["binding_id"],
               "requested_rule_id" => progress["model_rule_id"],
               "destination" => "https://api.model.example",
               "header" => "authorization",
               "header_prefix" => "Bearer "
             },
             rpc_options(state, :declare_brokered_https_rule)
           ) do
        {:ok, %{"rule_id" => rule_id}} ->
          if rule_id == progress["model_rule_id"] do
            {:ok, Map.put(progress, "brokered_rule_declared", true)}
          else
            {:error, :model_rule_id_mismatch, progress}
          end

        {:error, reason} ->
          {:error, reason, progress}
      end
    end
  end

  defp ensure_ui_installation(identity, state, release_agent, descriptor, repository, progress) do
    if progress["installation_id"] && progress["generation_id"] do
      {:ok,
       %{
         "installation_id" => progress["installation_id"],
         "generation_id" => progress["generation_id"]
       }, progress}
    else
      case rpc_client(state).install_ui(
             identity,
             state.data.project["organization_id"],
             {:repository, repository["repository_id"]},
             release_agent["release_id"],
             descriptor["key"],
             acknowledge_repository_git_access: true,
             idempotency_key: step_key(state, :install_ui)
           ) do
        {:ok, installation} ->
          next_progress =
            Map.merge(progress, %{
              "installation_id" => installation["installation_id"],
              "generation_id" => installation["generation_id"]
            })

          {:ok, installation, next_progress}

        {:error, reason} ->
          {:error, reason, progress}
      end
    end
  end

  defp revise_session_capability(client, identity, state, imported, release_agent, repository) do
    bindings =
      (release_agent["capability_requirements"] || [])
      |> Enum.filter(&(&1["slot_key"] == "session" and &1["resource_kind"] == "repository"))
      |> Enum.map(fn requirement ->
        %{
          "slot_key" => "session",
          "resource_kind" => "repository",
          "resource_id" => repository["repository_id"],
          "granted_operations" => requirement["required_operations"] || []
        }
      end)

    if length(bindings) != 1 or not required_git_operations?(hd(bindings)["granted_operations"]) do
      {:error, :missing_repository_capability}
    else
      client.revise_capabilities(
        identity,
        imported["instance_id"],
        imported["revision_id"],
        bindings,
        rpc_options(state, :revise_capabilities)
      )
    end
  end

  defp handoff(
         client,
         identity,
         %{"installation_id" => installation_id, "generation_id" => generation_id},
         route
       ) do
    client.create_ui_browser_handoff(
      identity,
      installation_id,
      generation_id,
      route,
      on_success: fn handoff, secret -> UIBrowser.bootstrap_url(handoff, secret, "light", []) end
    )
  end

  defp find_release_agent(catalog, id) do
    case Enum.find(catalog, &(&1["id"] == id)) do
      nil -> {:error, :release_unavailable}
      release_agent -> {:ok, release_agent}
    end
  end

  defp find_import(imports, id) do
    case Enum.find(imports, &(&1["id"] == id)) do
      nil -> {:error, :model_import_unavailable}
      import -> {:ok, import}
    end
  end

  defp session_ui_descriptor(client, identity, release_agent) do
    with release_id when is_binary(release_id) <- release_agent["release_id"],
         {:ok, release} <- client.get_release(identity, release_id),
         descriptor when is_map(descriptor) <-
           Enum.find(release["ui_descriptors"] || [], &compatible_session_ui?/1) do
      {:ok, descriptor}
    else
      nil -> {:error, :missing_session_ui}
      _invalid -> {:error, :missing_session_ui}
    end
  end

  defp compatible_session_ui?(descriptor) do
    descriptor["scope"] == "repository" and
      descriptor["presentation"] == "full_page" and
      descriptor["repository_git_access"] == "read_write" and
      is_binary(descriptor["key"]) and descriptor["key"] != "" and
      is_binary(descriptor["route_base"]) and descriptor["route_base"] != "" and
      is_map(descriptor["static_content"])
  end

  defp matching_attempt?(
         %{"setup_config" => expected},
         attributes,
         release_agent,
         model_import,
         descriptor,
         model_rule_id
       )
       when is_map(expected) do
    if expected ==
         setup_config(attributes, release_agent, model_import, descriptor, model_rule_id) do
      :ok
    else
      {:error, :attempt_conflict}
    end
  end

  defp matching_attempt?(
         _progress,
         _attributes,
         _release_agent,
         _model_import,
         _descriptor,
         _model_rule_id
       ),
       do: :ok

  defp valid_attempt_id?(state, %{"setup_attempt_id" => submitted}) when is_binary(submitted) do
    if submitted == Map.fetch!(state.data, :__attempt_id),
      do: :ok,
      else: {:error, :attempt_conflict}
  end

  defp valid_attempt_id?(_state, _attributes), do: {:error, :attempt_conflict}

  defp setup_config(attributes, release_agent, model_import, descriptor, model_rule_id) do
    %{
      "repository_name" => attributes["repository_name"] || "",
      "default_branch" => "main",
      "instance_name" => attributes["instance_name"] || "Session chat",
      "release_agent_id" => release_agent["id"],
      "release_id" => release_agent["release_id"],
      "model_rule_id" => model_rule_id,
      "model_import_id" => model_import["id"],
      "ui_key" => descriptor["key"],
      "ui_route" => descriptor["route_base"]
    }
  end

  defp rpc_options(state, step), do: [idempotency_key: step_key(state, step)]

  defp rpc_client(state), do: Map.get(state.data, :__client, Client)

  defp step_key(state, step),
    do: Map.fetch!(state.data, :__attempt_id) <> ":" <> Atom.to_string(step)

  defp compatible_release?(release_agent) do
    parameters = release_agent["parameter_schema"] || []
    slots = release_agent["secret_slot_schema"] || []
    requirements = release_agent["capability_requirements"] || []

    Enum.all?(parameters, fn parameter ->
      not parameter["required"] or parameter["name"] == "model_rule_id"
    end) and
      Enum.any?(parameters, &(&1["name"] == "model_rule_id" and &1["required"] == true)) and
      Enum.any?(slots, fn slot ->
        slot["key"] == "model" and slot["required"] == true and
          "brokered" in (slot["delivery_modes"] || []) and
          "normal" in (slot["phases"] || []) and
          "api.model.example" in (slot["destinations"] || [])
      end) and
      Enum.any?(requirements, fn requirement ->
        requirement["slot_key"] == "session" and requirement["resource_kind"] == "repository" and
          required_git_operations?(requirement["required_operations"] || [])
      end)
  end

  defp compatible_model_import?(import) do
    policy = import

    if "brokered" in (policy["delivery_modes"] || []) and
         "normal" in (policy["phases"] || []) and
         "api.model.example" in (policy["destinations"] || []) do
      :ok
    else
      {:error, :incompatible_model_import}
    end
  end

  defp required_git_operations?(operations),
    do: "git_read" in operations and "update_ref" in operations

  defp acknowledge_git?(%{"acknowledge_repository_git_access" => "true"}), do: :ok
  defp acknowledge_git?(_attributes), do: {:error, :git_acknowledgement_required}
  defp exact_main_branch?(%{"default_branch" => "main"}), do: :ok
  defp exact_main_branch?(_attributes), do: {:error, :invalid_branch}

  defp selected_policy(release_agent) do
    ceiling = get_in(release_agent, ["runtime_contract", "policy_ceiling"]) || %{}

    %{
      "vcpus" => ceiling["vcpus"] || 1,
      "memory_mib" => ceiling["memory_mib"] || 512,
      "network" => ceiling["network"] || "broker_only"
    }
  end

  defp presentation_status(%{status: status}) when status in [:ready, :submitting], do: :ready

  defp presentation_status(%{status: :error, data: %{project: project}})
       when is_map(project),
       do: :ready

  defp presentation_status(%{status: status}) when status in [:initial, :loading, :stale],
    do: :loading

  defp presentation_status(%{status: :reconnecting}), do: :reconnecting
  defp presentation_status(%{status: :access_revoked}), do: :error
  defp presentation_status(_state), do: :error

  defp present_error(%HephaestusWeb.RPC.Error{} = error),
    do: HephaestusWeb.RPC.Error.present(error)

  defp present_error(:git_acknowledgement_required),
    do: "Confirm repository Git access before creating the session."

  defp present_error(:invalid_branch), do: "Session chat uses the exact main branch."
  defp present_error(:model_import_unavailable), do: "Choose an authorized brokered model import."

  defp present_error(:incompatible_model_import),
    do: "Choose an authorized brokered model import for the selected release."

  defp present_error(:model_rule_id_mismatch),
    do: "The model rule declaration did not match this setup attempt."

  defp present_error(:missing_session_ui),
    do: "The selected release has no authorized repository session UI."

  defp present_error(:attempt_conflict),
    do:
      "This setup attempt already has resources; keep its original choices or start a new attempt."

  defp present_error(:missing_repository_capability),
    do: "The selected release has no compatible session repository capability."

  defp present_error(:release_unavailable), do: "The selected release is no longer available."
  defp present_error(_reason), do: "Session chat could not be created."

  defp progress_summary(progress),
    do: progress |> Map.keys() |> Enum.sort() |> Enum.join(", ")
end
