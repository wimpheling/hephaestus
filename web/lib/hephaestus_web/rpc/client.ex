defmodule HephaestusWeb.RPC.Client do
  @moduledoc """
  Domain-oriented Phoenix client built only from generated protobuf messages
  and generated native-gRPC stubs.
  """

  alias Hephaestus.Common.V1.{
    NetworkPolicy,
    OpaqueId,
    PageRequest,
    ParameterValue,
    RequestContext,
    RuntimePolicy
  }

  alias Hephaestus.Identity.V1.{IdentityService, ResolveIdentityRequest}

  alias Hephaestus.Builder.V1.{
    BuilderCatalogService,
    GetBuilderImageRequest,
    ListBuilderImagesRequest,
    ValidateAgentConfigRequest
  }

  alias Hephaestus.Build.V1.{
    BuildService,
    GetBuildRequest,
    ListBuildsRequest,
    RequestBuildRequest
  }

  alias Hephaestus.Instance.V1.{
    AgentInstanceService,
    BindSecretRequest,
    CreateAttachmentRequest,
    CreateUpdateRequest,
    GetInstanceRequest,
    ImportAgentRequest,
    RecoverUpdateRequest,
    RecoveryAction,
    RefSelector,
    RemoveAttachmentRequest,
    ReviseInstanceRequest,
    SetAttachmentEnabledRequest,
    TriggerPolicy
  }

  alias Hephaestus.Organization.V1.{
    GetOrganizationRequest,
    ListOrganizationsRequest,
    ListOrganizationProjectsRequest,
    ListOrganizationRepositoriesRequest,
    OrganizationService
  }

  alias Hephaestus.Project.V1.{
    CreateProjectRequest,
    GetProjectRequest,
    ListImportableReleaseAgentsRequest,
    ListProjectInstancesRequest,
    ListProjectRepositoriesRequest,
    ProjectService
  }

  alias Hephaestus.Release.V1.{
    GetReleaseRequest,
    ListRepositoryReleasesRequest,
    PublishReleaseRequest,
    ReleaseService,
    SetDraftVersionRequest
  }

  alias Hephaestus.Repository.V1.{
    CreateRepositoryRequest,
    GetRepositoryRequest,
    ListRepositoryInstancesRequest,
    RepositoryService
  }

  alias Hephaestus.RepositoryBrowser.V1.{
    GetFileRequest,
    GetTreeRequest,
    ListBranchesRequest,
    ListCommitsRequest,
    RepositoryBrowserService
  }

  alias Hephaestus.Run.V1.{
    GetRunRequest,
    ListProjectRunsRequest,
    RequestControlRequest,
    RunControlKind,
    RunControlTarget,
    RunService
  }

  alias Hephaestus.Secret.V1.{
    AcceptSecretImportRequest,
    CreateSecretRequest,
    DeliveryMode,
    DeliveryPhase,
    GetProjectSecretAuthorityRequest,
    GrantSecretRequest,
    ListOrganizationSecretGrantsRequest,
    ListOrganizationSecretsRequest,
    ListProjectSecretsRequest,
    PurgeSecretRequest,
    RevokeSecretRequest,
    RotateSecretRequest,
    SecretOwner,
    SecretPolicy,
    SecretService,
    SecretTarget,
    SetSecretEnabledRequest,
    SecretValue
  }

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.{Error, Invoke, Projection, UUID}

  @page_size 100
  @list_organizations "/hephaestus.organization.v1.OrganizationService/ListOrganizations"
  @create_secret "/hephaestus.secret.v1.SecretService/CreateSecret"

  @doc "Resolves a verified OIDC subject using bootstrap-only mediator authority."
  def resolve_identity(issuer, %{"sub" => subject} = claims)
      when is_binary(issuer) and is_binary(subject) do
    display_name =
      claims["name"] || claims["preferred_username"] || claims["email"] || subject

    attributes = %{
      issuer: issuer,
      subject: subject,
      display_name: display_name,
      email: claims["email"] || "",
      email_verified: claims["email_verified"] == true
    }

    {context, request_id} = request_context()
    request = struct!(ResolveIdentityRequest, Map.put(attributes, :context, context))

    case Invoke.bootstrap_unary(
           issuer,
           attributes,
           "/hephaestus.identity.v1.IdentityService/ResolveIdentity",
           request,
           &IdentityService.Stub.resolve_identity/3,
           request_id: request_id,
           maximum_request_bytes: 16_384,
           maximum_response_bytes: 4_096
         ) do
      {:ok, response} ->
        {:ok,
         %Identity{
           user_id: response.user_id.value,
           issuer: issuer,
           subject: subject,
           display_name: response.display_name
         }}

      {:error, error} ->
        {:error, error}
    end
  end

  def resolve_identity(_issuer, _claims), do: {:error, Error.local(:unauthenticated)}

  @doc "Lists every organization visible to the current user in stable server order."
  @spec list_organizations(Identity.t()) :: {:ok, [map()]} | {:error, term()}
  def list_organizations(%Identity{} = identity) do
    paginate(
      identity,
      @list_organizations,
      fn page -> %ListOrganizationsRequest{page: page} end,
      &OrganizationService.Stub.list_organizations/3,
      :organizations
    )
  end

  def get_organization(identity, organization_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.organization.v1.OrganizationService/GetOrganization",
        %GetOrganizationRequest{organization_id: id(organization_id)},
        &OrganizationService.Stub.get_organization/3,
        :organization
      )

  def list_repositories(identity, organization_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.organization.v1.OrganizationService/ListOrganizationRepositories",
        ListOrganizationRepositoriesRequest,
        :organization_id,
        organization_id,
        &OrganizationService.Stub.list_organization_repositories/3,
        :repositories
      )

  def list_projects(identity, organization_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.organization.v1.OrganizationService/ListOrganizationProjects",
        ListOrganizationProjectsRequest,
        :organization_id,
        organization_id,
        &OrganizationService.Stub.list_organization_projects/3,
        :projects
      )

  def get_project(identity, project_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.project.v1.ProjectService/GetProject",
        %GetProjectRequest{project_id: id(project_id)},
        &ProjectService.Stub.get_project/3,
        :project
      )

  def list_builder_images(identity),
    do:
      unary_projected(
        identity,
        "/hephaestus.builder.v1.BuilderCatalogService/ListBuilderImages",
        %ListBuilderImagesRequest{},
        &BuilderCatalogService.Stub.list_builder_images/3,
        nil,
        maximum_response_bytes: 1_048_576
      )

  def get_builder_image(identity, builder_image_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.builder.v1.BuilderCatalogService/GetBuilderImage",
        %GetBuilderImageRequest{builder_image_id: id(builder_image_id)},
        &BuilderCatalogService.Stub.get_builder_image/3,
        :builder_image,
        maximum_response_bytes: 65_536
      )

  def validate_agent_config(identity, agent_toml) when is_binary(agent_toml),
    do:
      unary_projected(
        identity,
        "/hephaestus.builder.v1.BuilderCatalogService/ValidateAgentConfig",
        %ValidateAgentConfigRequest{agent_toml: agent_toml},
        &BuilderCatalogService.Stub.validate_agent_config/3,
        nil,
        maximum_request_bytes: 65_536,
        maximum_response_bytes: 16_384
      )

  def create_project(identity, organization_id, name, description \\ ""),
    do:
      mutation(
        identity,
        "/hephaestus.project.v1.ProjectService/CreateProject",
        CreateProjectRequest,
        [organization_id: id(organization_id), name: name, description: description],
        &ProjectService.Stub.create_project/3
      )

  def list_project_repositories(identity, project_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.project.v1.ProjectService/ListProjectRepositories",
        ListProjectRepositoriesRequest,
        :project_id,
        project_id,
        &ProjectService.Stub.list_project_repositories/3,
        :repositories
      )

  def list_project_instances(identity, project_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.project.v1.ProjectService/ListProjectInstances",
        ListProjectInstancesRequest,
        :project_id,
        project_id,
        &ProjectService.Stub.list_project_instances/3,
        :instances
      )

  def list_importable_release_agents(identity, project_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.project.v1.ProjectService/ListImportableReleaseAgents",
        ListImportableReleaseAgentsRequest,
        :project_id,
        project_id,
        &ProjectService.Stub.list_importable_release_agents/3,
        :release_agents
      )

  def list_project_runs(identity, project_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.run.v1.RunService/ListProjectRuns",
        ListProjectRunsRequest,
        :project_id,
        project_id,
        &RunService.Stub.list_project_runs/3,
        :runs
      )

  def list_project_secrets(identity, project_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.secret.v1.SecretService/ListProjectSecrets",
        ListProjectSecretsRequest,
        :project_id,
        project_id,
        &SecretService.Stub.list_project_secrets/3,
        :secrets
      )

  def list_organization_secrets(identity, organization_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.secret.v1.SecretService/ListOrganizationSecrets",
        ListOrganizationSecretsRequest,
        :organization_id,
        organization_id,
        &SecretService.Stub.list_organization_secrets/3,
        :secrets
      )

  def list_organization_secret_grants(identity, organization_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.secret.v1.SecretService/ListOrganizationSecretGrants",
        ListOrganizationSecretGrantsRequest,
        :organization_id,
        organization_id,
        &SecretService.Stub.list_organization_secret_grants/3,
        :grants
      )

  def list_project_secret_authority(identity, project_id) do
    authority_pages(identity, project_id, "", "", MapSet.new(), [], [])
  end

  def get_repository(identity, repository_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.repository.v1.RepositoryService/GetRepository",
        %GetRepositoryRequest{repository_id: id(repository_id)},
        &RepositoryService.Stub.get_repository/3,
        :repository
      )

  def create_repository(
        identity,
        project_id,
        name,
        default_branch,
        is_public,
        agent_runs_enabled
      ),
      do:
        mutation(
          identity,
          "/hephaestus.repository.v1.RepositoryService/CreateRepository",
          CreateRepositoryRequest,
          [
            project_id: id(project_id),
            name: name,
            default_branch: default_branch,
            is_public: is_public,
            agent_runs_enabled: agent_runs_enabled
          ],
          &RepositoryService.Stub.create_repository/3
        )

  def list_builds(identity, repository_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.build.v1.BuildService/ListBuilds",
        ListBuildsRequest,
        :repository_id,
        repository_id,
        &BuildService.Stub.list_builds/3,
        :builds
      )

  def get_build(identity, build_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.build.v1.BuildService/GetBuild",
        %GetBuildRequest{build_id: id(build_id)},
        &BuildService.Stub.get_build/3,
        :build
      )

  def request_build(
        identity,
        repository_id,
        source_commit,
        build_definition_hash,
        configuration_hash
      ),
      do:
        mutation(
          identity,
          "/hephaestus.build.v1.BuildService/RequestBuild",
          RequestBuildRequest,
          [
            repository_id: id(repository_id),
            source_commit: source_commit,
            build_definition_hash: build_definition_hash,
            configuration_hash: configuration_hash
          ],
          &BuildService.Stub.request_build/3
        )

  def list_repository_releases(identity, repository_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.release.v1.ReleaseService/ListRepositoryReleases",
        ListRepositoryReleasesRequest,
        :repository_id,
        repository_id,
        &ReleaseService.Stub.list_repository_releases/3,
        :releases
      )

  def get_release(identity, release_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.release.v1.ReleaseService/GetRelease",
        %GetReleaseRequest{release_id: id(release_id)},
        &ReleaseService.Stub.get_release/3,
        :release,
        maximum_response_bytes: 8_388_608
      )

  def set_draft_version(identity, release_id, version),
    do:
      mutation(
        identity,
        "/hephaestus.release.v1.ReleaseService/SetDraftVersion",
        SetDraftVersionRequest,
        [release_id: id(release_id), version: version],
        &ReleaseService.Stub.set_draft_version/3
      )

  def publish_release(identity, release_id),
    do:
      mutation(
        identity,
        "/hephaestus.release.v1.ReleaseService/PublishRelease",
        PublishReleaseRequest,
        [release_id: id(release_id)],
        &ReleaseService.Stub.publish_release/3
      )

  def list_repository_instances(identity, repository_id),
    do:
      paged_by_id(
        identity,
        "/hephaestus.repository.v1.RepositoryService/ListRepositoryInstances",
        ListRepositoryInstancesRequest,
        :repository_id,
        repository_id,
        &RepositoryService.Stub.list_repository_instances/3,
        :attachments
      )

  def get_instance(identity, instance_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
        %GetInstanceRequest{instance_id: id(instance_id)},
        &AgentInstanceService.Stub.get_instance/3,
        :instance,
        maximum_response_bytes: 4_194_304
      )

  def get_run(identity, run_id),
    do:
      unary_projected(
        identity,
        "/hephaestus.run.v1.RunService/GetRun",
        %GetRunRequest{run_id: id(run_id)},
        &RunService.Stub.get_run/3,
        :run,
        maximum_response_bytes: 8_388_608
      )

  def branches(identity, repository_id) do
    with {:ok, branches} <-
           paged_by_id(
             identity,
             "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListBranches",
             ListBranchesRequest,
             :repository_id,
             repository_id,
             &RepositoryBrowserService.Stub.list_branches/3,
             :branches
           ) do
      {:ok, Enum.map(branches, &atom_keys/1)}
    end
  end

  def commits(identity, repository_id, branch),
    do:
      browser_collection(
        identity,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListCommits",
        fn page ->
          %ListCommitsRequest{repository_id: id(repository_id), branch: branch, page: page}
        end,
        &RepositoryBrowserService.Stub.list_commits/3,
        :commits
      )

  def tree(identity, repository_id, branch),
    do:
      browser_collection(
        identity,
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/GetTree",
        fn page ->
          %GetTreeRequest{repository_id: id(repository_id), branch: branch, page: page}
        end,
        &RepositoryBrowserService.Stub.get_tree/3,
        :entries
      )

  def file(identity, repository_id, branch, path) do
    request = %GetFileRequest{repository_id: id(repository_id), branch: branch, path: path}

    case Invoke.unary(
           identity,
           "/hephaestus.repository_browser.v1.RepositoryBrowserService/GetFile",
           request,
           &RepositoryBrowserService.Stub.get_file/3,
           retry: :safe_query,
           maximum_response_bytes: 1_114_112
         ) do
      {:ok, response} ->
        projected = Projection.to_value(response)

        {:ok,
         %{
           entry: atom_keys(projected["entry"]),
           contents: projected["utf8_contents"],
           language: projected["language"]
         }}

      {:error, error} ->
        {:error, error}
    end
  end

  @doc "Creates one encrypted secret through the typed secret service."
  @spec create_secret(
          Identity.t(),
          :organization | :project,
          String.t(),
          String.t(),
          [String.t()],
          binary()
        ) :: {:ok, map()} | {:error, term()}
  def create_secret(%Identity{} = identity, owner_kind, owner_id, name, modes, value) do
    {context, request_id} = request_context()

    request = %CreateSecretRequest{
      context: context,
      owner: secret_owner(owner_kind, owner_id),
      name: name,
      allowed_delivery_modes: Enum.map(modes, &delivery_mode!/1),
      secret: %SecretValue{value: value}
    }

    Invoke.unary(identity, @create_secret, request, &SecretService.Stub.create_secret/3,
      request_id: request_id,
      maximum_request_bytes: 1_048_576,
      maximum_response_bytes: 4_096
    )
    |> project_response()
  end

  def rotate_secret(identity, secret_id, expected_version_id, value) do
    mutation(
      identity,
      "/hephaestus.secret.v1.SecretService/RotateSecret",
      RotateSecretRequest,
      [
        secret_id: id(secret_id),
        expected_active_version_id: id(expected_version_id),
        secret: %SecretValue{value: value}
      ],
      &SecretService.Stub.rotate_secret/3
    )
  end

  def revoke_secret(identity, secret_id),
    do:
      mutation(
        identity,
        "/hephaestus.secret.v1.SecretService/RevokeSecret",
        RevokeSecretRequest,
        [secret_id: id(secret_id)],
        &SecretService.Stub.revoke_secret/3
      )

  def set_secret_enabled(identity, secret_id, enabled),
    do:
      mutation(
        identity,
        "/hephaestus.secret.v1.SecretService/SetSecretEnabled",
        SetSecretEnabledRequest,
        [secret_id: id(secret_id), enabled: enabled],
        &SecretService.Stub.set_secret_enabled/3
      )

  def purge_secret(identity, secret_id),
    do:
      mutation(
        identity,
        "/hephaestus.secret.v1.SecretService/PurgeSecret",
        PurgeSecretRequest,
        [secret_id: id(secret_id)],
        &SecretService.Stub.purge_secret/3
      )

  def grant_secret(identity, secret_id, target_kind, target_id, policy, expires_at) do
    with {:ok, expiration} <- timestamp(expires_at) do
      mutation(
        identity,
        "/hephaestus.secret.v1.SecretService/GrantSecret",
        GrantSecretRequest,
        [
          secret_id: id(secret_id),
          target: secret_target(target_kind, target_id),
          policy: %SecretPolicy{
            delivery_modes: Enum.map(policy["delivery_modes"] || [], &delivery_mode!/1),
            phases: Enum.map(policy["phases"] || [], &delivery_phase!/1),
            destinations: policy["destinations"] || []
          },
          expires_at: expiration
        ],
        &SecretService.Stub.grant_secret/3
      )
    end
  end

  def accept_secret_import(identity, grant_id, target_kind, target_id, alias_name) do
    mutation(
      identity,
      "/hephaestus.secret.v1.SecretService/AcceptSecretImport",
      AcceptSecretImportRequest,
      [
        grant_id: id(grant_id),
        target: secret_target(target_kind, target_id),
        alias: alias_name
      ],
      &SecretService.Stub.accept_secret_import/3
    )
  end

  def import_agent(identity, project_id, release_agent_id, name, parameters, policy) do
    mutation(
      identity,
      "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
      ImportAgentRequest,
      [
        project_id: id(project_id),
        release_agent_id: id(release_agent_id),
        name: name,
        parameters: parameter_values(parameters),
        selected_policy: runtime_policy(policy)
      ],
      &AgentInstanceService.Stub.import_agent/3
    )
  end

  def create_attachment(identity, instance_id, repository_id, selector, trigger) do
    mutation(
      identity,
      "/hephaestus.instance.v1.AgentInstanceService/CreateAttachment",
      CreateAttachmentRequest,
      [
        instance_id: id(instance_id),
        repository_id: id(repository_id),
        ref_selector: ref_selector(selector),
        trigger_policy: trigger_policy!(trigger)
      ],
      &AgentInstanceService.Stub.create_attachment/3
    )
  end

  def set_attachment_enabled(identity, attachment_id, enabled),
    do:
      mutation(
        identity,
        "/hephaestus.instance.v1.AgentInstanceService/SetAttachmentEnabled",
        SetAttachmentEnabledRequest,
        [attachment_id: id(attachment_id), enabled: enabled],
        &AgentInstanceService.Stub.set_attachment_enabled/3
      )

  def remove_attachment(identity, attachment_id),
    do:
      mutation(
        identity,
        "/hephaestus.instance.v1.AgentInstanceService/RemoveAttachment",
        RemoveAttachmentRequest,
        [attachment_id: id(attachment_id)],
        &AgentInstanceService.Stub.remove_attachment/3
      )

  def revise_instance(identity, instance_id, expected_revision_id, parameters, policy) do
    mutation(
      identity,
      "/hephaestus.instance.v1.AgentInstanceService/ReviseInstance",
      ReviseInstanceRequest,
      [
        instance_id: id(instance_id),
        expected_revision_id: id(expected_revision_id),
        parameters: parameter_values(parameters),
        selected_policy: runtime_policy(policy)
      ],
      &AgentInstanceService.Stub.revise_instance/3
    )
  end

  def create_update(
        identity,
        instance_id,
        expected_revision_id,
        candidate_release_agent_id,
        parameters,
        policy
      ) do
    mutation(
      identity,
      "/hephaestus.instance.v1.AgentInstanceService/CreateUpdate",
      CreateUpdateRequest,
      [
        instance_id: id(instance_id),
        expected_revision_id: id(expected_revision_id),
        candidate_release_agent_id: id(candidate_release_agent_id),
        parameters: parameter_values(parameters),
        selected_policy: runtime_policy(policy)
      ],
      &AgentInstanceService.Stub.create_update/3
    )
  end

  def recover_update(identity, update_id, action),
    do:
      mutation(
        identity,
        "/hephaestus.instance.v1.AgentInstanceService/RecoverUpdate",
        RecoverUpdateRequest,
        [update_id: id(update_id), action: recovery_action!(action)],
        &AgentInstanceService.Stub.recover_update/3
      )

  def bind_secret(identity, attributes) do
    mutation(
      identity,
      "/hephaestus.instance.v1.AgentInstanceService/BindSecret",
      BindSecretRequest,
      [
        instance_id: id(Map.fetch!(attributes, "instance_id")),
        expected_revision_id: id(Map.fetch!(attributes, "expected_revision_id")),
        import_id: id(Map.fetch!(attributes, "import_id")),
        slot: Map.fetch!(attributes, "slot"),
        mode: delivery_mode!(Map.fetch!(attributes, "mode")),
        phases: Enum.map(attributes["phases"] || [], &delivery_phase!/1),
        attachment_ids: Enum.map(attributes["attachment_ids"] || [], &id/1),
        destinations: attributes["destinations"] || []
      ],
      &AgentInstanceService.Stub.bind_secret/3
    )
  end

  def create_control(identity, attributes) do
    {context, request_id} = request_context()
    kind = Map.fetch!(attributes, "kind")

    request = %RequestControlRequest{
      context: context,
      kind: run_control_kind!(kind),
      repository_id: id(Map.fetch!(attributes, "repository_id")),
      target: run_control_target(kind, attributes),
      reason: String.slice(attributes["reason"] || "", 0, 4_096)
    }

    case Invoke.unary(
           identity,
           "/hephaestus.run.v1.RunService/RequestControl",
           request,
           &RunService.Stub.request_control/3,
           request_id: request_id,
           maximum_request_bytes: 16_384,
           maximum_response_bytes: 8_192
         ) do
      {:ok, response} ->
        {:ok,
         %{
           "control_request_id" => Projection.to_value(response.control_request_id),
           "receipt" => Projection.to_value(response.receipt)
         }}

      {:error, error} ->
        {:error, error}
    end
  end

  defp unary_projected(identity, audience, request, stub_call, field, options \\ []) do
    invoke_options = Keyword.put_new(options, :retry, :safe_query)

    case Invoke.unary(identity, audience, request, stub_call, invoke_options) do
      {:ok, response} ->
        value = if field, do: Map.fetch!(response, field), else: response
        {:ok, Projection.to_value(value)}

      {:error, error} ->
        {:error, error}
    end
  end

  defp mutation(identity, audience, request_module, attributes, stub_call) do
    {context, request_id} = request_context()
    request = struct!(request_module, [{:context, context} | attributes])

    Invoke.unary(identity, audience, request, stub_call,
      request_id: request_id,
      maximum_request_bytes: 1_048_576,
      maximum_response_bytes: 65_536
    )
    |> project_response()
  end

  defp paged_by_id(identity, audience, request_module, id_field, value, stub_call, field) do
    paginate(
      identity,
      audience,
      fn page -> struct!(request_module, [{id_field, id(value)}, {:page, page}]) end,
      stub_call,
      field
    )
  end

  defp browser_collection(identity, audience, request_builder, stub_call, field) do
    browser_collection(
      identity,
      audience,
      request_builder,
      stub_call,
      field,
      "",
      MapSet.new(),
      [],
      nil
    )
  end

  defp authority_pages(
         identity,
         project_id,
         grants_token,
         imports_token,
         seen,
         grants,
         imports
       ) do
    request = %GetProjectSecretAuthorityRequest{
      project_id: id(project_id),
      grants_page: %PageRequest{page_size: @page_size, page_token: grants_token},
      imports_page: %PageRequest{page_size: @page_size, page_token: imports_token}
    }

    case Invoke.unary(
           identity,
           "/hephaestus.secret.v1.SecretService/GetProjectSecretAuthority",
           request,
           &SecretService.Stub.get_project_secret_authority/3,
           retry: :safe_query
         ) do
      {:ok, response} ->
        next_grants = next_page_token(response.grants_page)
        next_imports = next_page_token(response.imports_page)
        next_pair = {next_grants, next_imports}
        grants = grants ++ response.grants
        imports = imports ++ response.imports

        if next_pair == {"", ""} or MapSet.member?(seen, next_pair) do
          {:ok,
           %{
             "grants" => grants |> unique_messages() |> Projection.to_value(),
             "imports" => imports |> unique_messages() |> Projection.to_value()
           }}
        else
          authority_pages(
            identity,
            project_id,
            next_grants,
            next_imports,
            MapSet.put(seen, next_pair),
            grants,
            imports
          )
        end

      {:error, error} ->
        {:error, error}
    end
  end

  defp browser_collection(
         identity,
         audience,
         request_builder,
         stub_call,
         field,
         token,
         seen,
         pages,
         selected_branch
       ) do
    request = request_builder.(%PageRequest{page_size: @page_size, page_token: token})

    case Invoke.unary(identity, audience, request, stub_call, retry: :safe_query) do
      {:ok, response} ->
        values = Map.fetch!(response, field)
        selected_branch = selected_branch || response.selected_branch
        next_token = response.page && response.page.next_page_token
        accumulated = [values | pages]

        if next_token in [nil, ""] or MapSet.member?(seen, next_token) do
          items = accumulated |> Enum.reverse() |> List.flatten() |> Projection.to_value()

          {:ok,
           %{
             field => Enum.map(items, &atom_keys/1),
             :branch => selected_branch |> Projection.to_value() |> atom_keys()
           }}
        else
          browser_collection(
            identity,
            audience,
            request_builder,
            stub_call,
            field,
            next_token,
            MapSet.put(seen, next_token),
            accumulated,
            selected_branch
          )
        end

      {:error, error} ->
        {:error, error}
    end
  end

  defp paginate(identity, audience, request_builder, stub_call, field) do
    paginate(identity, audience, request_builder, stub_call, field, "", MapSet.new(), [])
  end

  defp paginate(identity, audience, request_builder, stub_call, field, token, seen, pages) do
    request = request_builder.(%PageRequest{page_size: @page_size, page_token: token})

    case Invoke.unary(identity, audience, request, stub_call, retry: :safe_query) do
      {:ok, response} ->
        values = Map.fetch!(response, field)
        next_token = response.page && response.page.next_page_token
        accumulated = [values | pages]

        if next_token in [nil, ""] or MapSet.member?(seen, next_token) do
          {:ok, accumulated |> Enum.reverse() |> List.flatten() |> Projection.to_value()}
        else
          paginate(
            identity,
            audience,
            request_builder,
            stub_call,
            field,
            next_token,
            MapSet.put(seen, next_token),
            accumulated
          )
        end

      {:error, error} ->
        {:error, error}
    end
  end

  defp unique_messages(messages) do
    Enum.uniq_by(messages, fn message ->
      case Map.get(message, :id) do
        %OpaqueId{value: value} -> value
        _other -> message
      end
    end)
  end

  defp next_page_token(nil), do: ""
  defp next_page_token(page), do: page.next_page_token || ""

  defp request_context do
    request_id = UUID.generate()
    idempotency_key = UUID.generate()

    {%RequestContext{
       request_id: id(request_id),
       idempotency_key: idempotency_key
     }, request_id}
  end

  defp secret_owner(:organization, owner_id),
    do: %SecretOwner{owner: {:organization_id, id(owner_id)}}

  defp secret_owner(:project, owner_id),
    do: %SecretOwner{owner: {:project_id, id(owner_id)}}

  defp delivery_mode!("raw"), do: DeliveryMode.value(:DELIVERY_MODE_RAW)
  defp delivery_mode!("brokered"), do: DeliveryMode.value(:DELIVERY_MODE_BROKERED)
  defp delivery_phase!("normal"), do: DeliveryPhase.value(:DELIVERY_PHASE_NORMAL)
  defp delivery_phase!("update"), do: DeliveryPhase.value(:DELIVERY_PHASE_UPDATE)

  defp secret_target("project", target_id),
    do: %SecretTarget{target: {:project_id, id(target_id)}}

  defp secret_target("repository", target_id),
    do: %SecretTarget{target: {:repository_id, id(target_id)}}

  defp ref_selector(%{"type" => "exact", "value" => value}),
    do: %RefSelector{selector: {:exact, value}}

  defp ref_selector(%{"type" => "prefix", "value" => value}),
    do: %RefSelector{selector: {:prefix, value}}

  defp trigger_policy!("manual"), do: TriggerPolicy.value(:TRIGGER_POLICY_MANUAL)
  defp trigger_policy!("push"), do: TriggerPolicy.value(:TRIGGER_POLICY_PUSH)

  defp trigger_policy!("push_and_manual"),
    do: TriggerPolicy.value(:TRIGGER_POLICY_PUSH_AND_MANUAL)

  defp recovery_action!("retry"), do: RecoveryAction.value(:RECOVERY_ACTION_RETRY)
  defp recovery_action!("reject"), do: RecoveryAction.value(:RECOVERY_ACTION_REJECT)
  defp recovery_action!("resume"), do: RecoveryAction.value(:RECOVERY_ACTION_RESUME)

  defp runtime_policy(policy) do
    %RuntimePolicy{
      vcpus: policy["vcpus"],
      memory_mib: policy["memory_mib"],
      network: network_policy!(policy["network"])
    }
  end

  defp network_policy!("disabled"), do: NetworkPolicy.value(:NETWORK_POLICY_DISABLED)
  defp network_policy!("broker_only"), do: NetworkPolicy.value(:NETWORK_POLICY_BROKER_ONLY)
  defp network_policy!("egress"), do: NetworkPolicy.value(:NETWORK_POLICY_EGRESS)

  defp parameter_values(parameters) do
    Enum.map(parameters, fn
      {name, value} when is_boolean(value) ->
        %ParameterValue{name: name, value: {:boolean_value, value}}

      {name, value} when is_integer(value) ->
        %ParameterValue{name: name, value: {:integer_value, value}}

      {name, value} when is_binary(value) ->
        %ParameterValue{name: name, value: {:string_value, value}}
    end)
  end

  defp timestamp(nil), do: {:ok, nil}

  defp timestamp(value) do
    normalized = if String.length(value) == 16, do: value <> ":00", else: value

    case NaiveDateTime.from_iso8601(normalized) do
      {:ok, naive} ->
        datetime = DateTime.from_naive!(naive, "Etc/UTC")
        {:ok, %Google.Protobuf.Timestamp{seconds: DateTime.to_unix(datetime), nanos: 0}}

      {:error, _reason} ->
        {:error, Error.local(:invalid)}
    end
  end

  defp run_control_kind!("cancel_run"), do: RunControlKind.value(:RUN_CONTROL_KIND_CANCEL)
  defp run_control_kind!("retry_run"), do: RunControlKind.value(:RUN_CONTROL_KIND_RETRY)

  defp run_control_kind!("approve_result"),
    do: RunControlKind.value(:RUN_CONTROL_KIND_APPROVE_RESULT)

  defp run_control_kind!("reject_result"),
    do: RunControlKind.value(:RUN_CONTROL_KIND_REJECT_RESULT)

  defp run_control_target(kind, attributes) when kind in ["cancel_run", "retry_run"],
    do: %RunControlTarget{target: {:run_id, id(Map.fetch!(attributes, "run_id"))}}

  defp run_control_target(_kind, attributes),
    do: %RunControlTarget{target: {:proposal_id, id(Map.fetch!(attributes, "proposal_id"))}}

  defp id(value), do: %OpaqueId{value: value}

  defp atom_keys(map) when is_map(map) do
    Map.new(map, fn {key, value} -> {String.to_existing_atom(key), value} end)
  end

  defp project_response({:ok, response}), do: {:ok, Projection.to_value(response)}
  defp project_response({:error, error}), do: {:error, error}
end
