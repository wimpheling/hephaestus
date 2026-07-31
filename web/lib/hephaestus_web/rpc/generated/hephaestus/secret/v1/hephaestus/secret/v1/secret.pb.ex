defmodule Hephaestus.Secret.V1.DeliveryMode do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.secret.v1.DeliveryMode",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:DELIVERY_MODE_UNSPECIFIED, 0)
  field(:DELIVERY_MODE_RAW, 1)
  field(:DELIVERY_MODE_BROKERED, 2)
end

defmodule Hephaestus.Secret.V1.DeliveryPhase do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.secret.v1.DeliveryPhase",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:DELIVERY_PHASE_UNSPECIFIED, 0)
  field(:DELIVERY_PHASE_NORMAL, 1)
  field(:DELIVERY_PHASE_UPDATE, 2)
end

defmodule Hephaestus.Secret.V1.SecretState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.secret.v1.SecretState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:SECRET_STATE_UNSPECIFIED, 0)
  field(:SECRET_STATE_ACTIVE, 1)
  field(:SECRET_STATE_DISABLED, 2)
  field(:SECRET_STATE_REVOKED, 3)
  field(:SECRET_STATE_PURGED, 4)
end

defmodule Hephaestus.Secret.V1.AuthorityState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.secret.v1.AuthorityState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:AUTHORITY_STATE_UNSPECIFIED, 0)
  field(:AUTHORITY_STATE_ACTIVE, 1)
  field(:AUTHORITY_STATE_REVOKED, 2)
  field(:AUTHORITY_STATE_EXPIRED, 3)
end

defmodule Hephaestus.Secret.V1.SecretOwner do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SecretOwner",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:owner, 0)

  field(:organization_id, 1,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "organizationId",
    oneof: 0
  )

  field(:project_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId", oneof: 0)
end

defmodule Hephaestus.Secret.V1.SecretTarget do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SecretTarget",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:target, 0)

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId", oneof: 0)

  field(:repository_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "repositoryId",
    oneof: 0
  )
end

defmodule Hephaestus.Secret.V1.SecretValue do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SecretValue",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:value, 1, type: :bytes, deprecated: false)
end

defmodule Hephaestus.Secret.V1.SecretPolicy do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SecretPolicy",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:delivery_modes, 1,
    repeated: true,
    type: Hephaestus.Secret.V1.DeliveryMode,
    json_name: "deliveryModes",
    enum: true
  )

  field(:phases, 2, repeated: true, type: Hephaestus.Secret.V1.DeliveryPhase, enum: true)
  field(:destinations, 3, repeated: true, type: :string)
end

defmodule Hephaestus.Secret.V1.SecretLastUse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SecretLastUse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:operation, 1, type: :string)
  field(:outcome, 2, type: :string)

  field(:delivery_mode, 3,
    type: Hephaestus.Secret.V1.DeliveryMode,
    json_name: "deliveryMode",
    enum: true
  )

  field(:occurred_at, 4, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
end

defmodule Hephaestus.Secret.V1.SecretSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SecretSummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:state, 3, type: Hephaestus.Secret.V1.SecretState, enum: true)

  field(:allowed_delivery_modes, 4,
    repeated: true,
    type: Hephaestus.Secret.V1.DeliveryMode,
    json_name: "allowedDeliveryModes",
    enum: true
  )

  field(:active_version_id, 5, type: Hephaestus.Common.V1.OpaqueId, json_name: "activeVersionId")
  field(:created_at, 6, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 7, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
  field(:active_version_sequence, 8, type: :int64, json_name: "activeVersionSequence")

  field(:active_version_created_at, 9,
    type: Google.Protobuf.Timestamp,
    json_name: "activeVersionCreatedAt"
  )

  field(:grant_count, 10, type: :int64, json_name: "grantCount")
  field(:import_count, 11, type: :int64, json_name: "importCount")
  field(:binding_count, 12, type: :int64, json_name: "bindingCount")
  field(:has_raw_binding, 13, type: :bool, json_name: "hasRawBinding")
  field(:can_rotate, 14, type: :bool, json_name: "canRotate")
  field(:can_manage_grants, 15, type: :bool, json_name: "canManageGrants")
  field(:can_revoke, 16, type: :bool, json_name: "canRevoke")
  field(:can_purge, 17, type: :bool, json_name: "canPurge")
  field(:last_use, 18, type: Hephaestus.Secret.V1.SecretLastUse, json_name: "lastUse")
end

defmodule Hephaestus.Secret.V1.GrantSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.GrantSummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:secret_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:secret_name, 3, type: :string, json_name: "secretName")
  field(:target, 4, type: Hephaestus.Secret.V1.SecretTarget)
  field(:target_name, 5, type: :string, json_name: "targetName")
  field(:policy, 6, type: Hephaestus.Secret.V1.SecretPolicy)
  field(:expires_at, 7, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
  field(:state, 8, type: Hephaestus.Secret.V1.AuthorityState, enum: true)
  field(:created_at, 9, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:import_count, 10, type: :int64, json_name: "importCount")
  field(:import_id, 11, type: Hephaestus.Common.V1.OpaqueId, json_name: "importId")
  field(:import_alias, 12, type: :string, json_name: "importAlias")

  field(:import_state, 13,
    type: Hephaestus.Secret.V1.AuthorityState,
    json_name: "importState",
    enum: true
  )
end

defmodule Hephaestus.Secret.V1.ImportSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.ImportSummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:alias, 2, type: :string)
  field(:target, 3, type: Hephaestus.Secret.V1.SecretTarget)
  field(:state, 4, type: Hephaestus.Secret.V1.AuthorityState, enum: true)
  field(:secret_id, 5, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:secret_name, 6, type: :string, json_name: "secretName")

  field(:secret_state, 7,
    type: Hephaestus.Secret.V1.SecretState,
    json_name: "secretState",
    enum: true
  )

  field(:policy, 8, type: Hephaestus.Secret.V1.SecretPolicy)
  field(:expires_at, 9, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
end

defmodule Hephaestus.Secret.V1.ListProjectSecretsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.ListProjectSecretsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Secret.V1.ListProjectSecretsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.ListProjectSecretsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secrets, 1, repeated: true, type: Hephaestus.Secret.V1.SecretSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Secret.V1.ListOrganizationSecretsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.ListOrganizationSecretsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Secret.V1.ListOrganizationSecretsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.ListOrganizationSecretsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secrets, 1, repeated: true, type: Hephaestus.Secret.V1.SecretSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Secret.V1.ListOrganizationSecretGrantsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.ListOrganizationSecretGrantsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Secret.V1.ListOrganizationSecretGrantsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.ListOrganizationSecretGrantsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:grants, 1, repeated: true, type: Hephaestus.Secret.V1.GrantSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Secret.V1.GetProjectSecretAuthorityRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.GetProjectSecretAuthorityRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:grants_page, 2, type: Hephaestus.Common.V1.PageRequest, json_name: "grantsPage")
  field(:imports_page, 3, type: Hephaestus.Common.V1.PageRequest, json_name: "importsPage")
end

defmodule Hephaestus.Secret.V1.GetProjectSecretAuthorityResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.GetProjectSecretAuthorityResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:grants, 1, repeated: true, type: Hephaestus.Secret.V1.GrantSummary)
  field(:imports, 2, repeated: true, type: Hephaestus.Secret.V1.ImportSummary)
  field(:grants_page, 3, type: Hephaestus.Common.V1.PageResponse, json_name: "grantsPage")
  field(:imports_page, 4, type: Hephaestus.Common.V1.PageResponse, json_name: "importsPage")
end

defmodule Hephaestus.Secret.V1.CreateSecretRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.CreateSecretRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:owner, 2, type: Hephaestus.Secret.V1.SecretOwner)
  field(:name, 3, type: :string)

  field(:allowed_delivery_modes, 4,
    repeated: true,
    type: Hephaestus.Secret.V1.DeliveryMode,
    json_name: "allowedDeliveryModes",
    enum: true
  )

  field(:secret, 5, type: Hephaestus.Secret.V1.SecretValue)
end

defmodule Hephaestus.Secret.V1.CreateSecretResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.CreateSecretResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secret_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:version_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "versionId")
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Secret.V1.RotateSecretRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.RotateSecretRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:secret_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")

  field(:expected_active_version_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedActiveVersionId"
  )

  field(:secret, 4, type: Hephaestus.Secret.V1.SecretValue)
end

defmodule Hephaestus.Secret.V1.RotateSecretResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.RotateSecretResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secret_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:version_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "versionId")
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Secret.V1.RevokeSecretRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.RevokeSecretRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:secret_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
end

defmodule Hephaestus.Secret.V1.SetSecretEnabledRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SetSecretEnabledRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:secret_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:enabled, 3, type: :bool)
end

defmodule Hephaestus.Secret.V1.PurgeSecretRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.PurgeSecretRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:secret_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
end

defmodule Hephaestus.Secret.V1.RevokeSecretResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.RevokeSecretResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secret_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:state, 2, type: Hephaestus.Secret.V1.SecretState, enum: true)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Secret.V1.SetSecretEnabledResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.SetSecretEnabledResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secret_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:state, 2, type: Hephaestus.Secret.V1.SecretState, enum: true)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Secret.V1.PurgeSecretResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.PurgeSecretResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secret_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:state, 2, type: Hephaestus.Secret.V1.SecretState, enum: true)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Secret.V1.GrantSecretRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.GrantSecretRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:secret_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:target, 3, type: Hephaestus.Secret.V1.SecretTarget)
  field(:policy, 4, type: Hephaestus.Secret.V1.SecretPolicy)
  field(:expires_at, 5, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
end

defmodule Hephaestus.Secret.V1.GrantSecretResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.GrantSecretResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:grant_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "grantId")
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Secret.V1.AcceptSecretImportRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.AcceptSecretImportRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:grant_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "grantId")
  field(:target, 3, type: Hephaestus.Secret.V1.SecretTarget)
  field(:alias, 4, type: :string)
end

defmodule Hephaestus.Secret.V1.AcceptSecretImportResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.secret.v1.AcceptSecretImportResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:import_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "importId")
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Secret.V1.SecretService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.secret.v1.SecretService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListProjectSecrets,
    Hephaestus.Secret.V1.ListProjectSecretsRequest,
    Hephaestus.Secret.V1.ListProjectSecretsResponse
  )

  rpc(
    :ListOrganizationSecrets,
    Hephaestus.Secret.V1.ListOrganizationSecretsRequest,
    Hephaestus.Secret.V1.ListOrganizationSecretsResponse
  )

  rpc(
    :ListOrganizationSecretGrants,
    Hephaestus.Secret.V1.ListOrganizationSecretGrantsRequest,
    Hephaestus.Secret.V1.ListOrganizationSecretGrantsResponse
  )

  rpc(
    :GetProjectSecretAuthority,
    Hephaestus.Secret.V1.GetProjectSecretAuthorityRequest,
    Hephaestus.Secret.V1.GetProjectSecretAuthorityResponse
  )

  rpc(
    :CreateSecret,
    Hephaestus.Secret.V1.CreateSecretRequest,
    Hephaestus.Secret.V1.CreateSecretResponse
  )

  rpc(
    :RotateSecret,
    Hephaestus.Secret.V1.RotateSecretRequest,
    Hephaestus.Secret.V1.RotateSecretResponse
  )

  rpc(
    :RevokeSecret,
    Hephaestus.Secret.V1.RevokeSecretRequest,
    Hephaestus.Secret.V1.RevokeSecretResponse
  )

  rpc(
    :SetSecretEnabled,
    Hephaestus.Secret.V1.SetSecretEnabledRequest,
    Hephaestus.Secret.V1.SetSecretEnabledResponse
  )

  rpc(
    :PurgeSecret,
    Hephaestus.Secret.V1.PurgeSecretRequest,
    Hephaestus.Secret.V1.PurgeSecretResponse
  )

  rpc(
    :GrantSecret,
    Hephaestus.Secret.V1.GrantSecretRequest,
    Hephaestus.Secret.V1.GrantSecretResponse
  )

  rpc(
    :AcceptSecretImport,
    Hephaestus.Secret.V1.AcceptSecretImportRequest,
    Hephaestus.Secret.V1.AcceptSecretImportResponse
  )
end

defmodule Hephaestus.Secret.V1.SecretService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Secret.V1.SecretService.Service
end
