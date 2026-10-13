//! Complete present graph or conservative positively qualified no-Git graph.

use super::scope::{self, ReleaseRequirement, StoredBinding, StoredSnapshot};
use crate::{StoredRow, storage};
use runtime_authority::ExpiredRuntimeHandoff;
use runtime_git_authority::{
    ExpiredRuntimeGitCeiling, ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Credential {
    #[sqlx(flatten)]
    stored: StoredRow,
    credential_hash: Vec<u8>,
}

pub(super) async fn observe(
    tx: &mut Transaction<'_, Postgres>,
    original: &ExpiredRuntimeHandoff,
    ceiling: ExpiredRuntimeGitCeiling,
) -> Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError> {
    let mut credentials: Vec<Credential> = sqlx::query_as(
        "SELECT runtime_session_id,authorization_snapshot_id,binding_id,repository_id,
                scope_hash,issuance_generation,expires_at,credential_hash
         FROM public.runtime_git_credentials WHERE runtime_session_id=$1 OR authorization_snapshot_id=$2 LIMIT 2")
        .bind(original.session().id.as_uuid()).bind(original.snapshot().id().as_uuid())
        .fetch_all(&mut **tx).await.map_err(storage)?;
    let snapshots: Vec<StoredSnapshot> = sqlx::query_as(
        "SELECT snapshot_id,instance_revision_id,binding_id,repository_id,expected_parent,
                    grammar_version,git_operations,ref_globs,changed_path_globs,branch_update_policy,
                    branch_create,branch_delete,tag_create,tag_update,tag_delete,
                    other_create,other_update,other_delete,request_bytes,pack_bytes,
                    object_count,ref_updates,exact_parent_required,normalized_hash
         FROM public.run_git_authority_snapshots WHERE snapshot_id=$1 LIMIT 2")
        .bind(original.snapshot().id().as_uuid()).fetch_all(&mut **tx).await.map_err(storage)?;
    match shape(credentials.len(), snapshots.len())? {
        GraphShape::Absent => {
            absent(tx, original).await?;
            Ok(ExpiredRuntimeGitHandoff::absent(ceiling))
        }
        GraphShape::Present => {
            let [snapshot] = snapshots.as_slice() else {
                return Err(RuntimeGitAuthorityError::IdentityMismatch);
            };
            let credential = credentials
                .pop()
                .ok_or(RuntimeGitAuthorityError::IdentityMismatch)?;
            present(tx, original, ceiling, snapshot, credential).await
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum GraphShape {
    Absent,
    Present,
}

pub(super) const fn shape(
    credentials: usize,
    snapshots: usize,
) -> Result<GraphShape, RuntimeGitAuthorityError> {
    match (credentials, snapshots) {
        (0, 0) => Ok(GraphShape::Absent),
        (1, 1) => Ok(GraphShape::Present),
        _ => Err(RuntimeGitAuthorityError::IdentityMismatch),
    }
}

async fn absent(
    tx: &mut Transaction<'_, Postgres>,
    original: &ExpiredRuntimeHandoff,
) -> Result<(), RuntimeGitAuthorityError> {
    // Complete original metadata, not a mutable actor grant or a missing credential.
    // Conservatively hold even an unused typed Git declaration/binding.
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM public.agent_git_capability_bindings WHERE instance_revision_id=$1),
                (SELECT count(*) FROM public.release_git_capability_ceilings WHERE release_agent_id=$2)")
        .bind(original.pins().revision()).bind(original.pins().release_agent())
        .fetch_one(&mut **tx).await.map_err(storage)?;
    let declared: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.release_capability_requirements WHERE release_agent_id=$1
          AND (required_operations||optional_operations)&&ARRAY['git_read','create_ref','update_ref',
              'force_update_ref','delete_ref','create_tag','delete_tag','trigger_run','manage_attachments']::text[])")
        .bind(original.pins().release_agent()).fetch_one(&mut **tx).await.map_err(storage)?;
    let copied_git = original.snapshot().bindings().any(|binding| {
        binding.granted_operations().any(|op| {
            matches!(
                op.as_str(),
                "git_read"
                    | "create_ref"
                    | "update_ref"
                    | "force_update_ref"
                    | "delete_ref"
                    | "create_tag"
                    | "delete_tag"
                    | "trigger_run"
                    | "manage_attachments"
            )
        })
    });
    qualify_absence(counts, declared, copied_git)
}

pub(super) fn qualify_absence(
    counts: (i64, i64),
    declared: bool,
    copied_git: bool,
) -> Result<(), RuntimeGitAuthorityError> {
    if counts == (0, 0) && !declared && !copied_git {
        Ok(())
    } else {
        Err(RuntimeGitAuthorityError::IdentityMismatch)
    }
}

async fn present(
    tx: &mut Transaction<'_, Postgres>,
    original: &ExpiredRuntimeHandoff,
    ceiling: ExpiredRuntimeGitCeiling,
    snapshot: &StoredSnapshot,
    credential: Credential,
) -> Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError> {
    let bindings: Vec<StoredBinding> = sqlx::query_as(
        "SELECT binding_id,instance_revision_id,requirement_id,grammar_version,git_operations,ref_globs,changed_path_globs,branch_update_policy,
                    branch_create,branch_delete,tag_create,tag_update,tag_delete,
                    other_create,other_update,other_delete,request_bytes,pack_bytes,
                    object_count,ref_updates,exact_parent_required,normalized_hash
         FROM public.agent_git_capability_bindings WHERE binding_id=$1 LIMIT 2")
        .bind(snapshot.binding_id).fetch_all(&mut **tx).await.map_err(storage)?;
    let [binding] = bindings.as_slice() else {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    };
    let releases: Vec<ReleaseRequirement> = sqlx::query_as(
        "SELECT ceiling.requirement_id,ceiling.release_agent_id,requirement.normalized_hash AS requirement_hash,
                (requirement.required_operations||requirement.optional_operations) AS declared_operations,
                requirement.resource_kind,ceiling.grammar_version,ceiling.git_operations,ceiling.ref_globs,ceiling.changed_path_globs,ceiling.branch_update_policy,ceiling.branch_create,ceiling.branch_delete,ceiling.tag_create,ceiling.tag_update,ceiling.tag_delete,ceiling.other_create,ceiling.other_update,ceiling.other_delete,ceiling.request_bytes,ceiling.pack_bytes,ceiling.object_count,ceiling.ref_updates,ceiling.exact_parent_required,ceiling.normalized_hash
         FROM public.release_git_capability_ceilings ceiling
         JOIN public.release_capability_requirements requirement ON requirement.id=ceiling.requirement_id
           AND requirement.release_agent_id=ceiling.release_agent_id
         WHERE ceiling.requirement_id=$1 LIMIT 2")
        .bind(binding.requirement_id).fetch_all(&mut **tx).await.map_err(storage)?;
    let [release] = releases.as_slice() else {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    };
    scope::qualify(snapshot, binding, release, original)?;
    parent(tx, original.pins().run(), snapshot).await?;
    if credential.stored.runtime_session_id != original.session().id.as_uuid()
        || credential.stored.authorization_snapshot_id != snapshot.snapshot_id
        || credential.stored.binding_id != snapshot.binding_id
        || credential.stored.repository_id != snapshot.repository_id
        || credential.stored.scope_hash != snapshot.shape.normalized_hash
    {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    }
    let verifier: [u8; 32] = credential
        .credential_hash
        .try_into()
        .map_err(|_| RuntimeGitAuthorityError::Persistence)?;
    // Conversion and closed construction enforce exact generation and expiry.
    ExpiredRuntimeGitHandoff::present(ceiling, credential.stored.try_into()?, verifier)
}

async fn parent(
    tx: &mut Transaction<'_, Postgres>,
    run: Uuid,
    snapshot: &StoredSnapshot,
) -> Result<(), RuntimeGitAuthorityError> {
    if !snapshot.shape.exact_parent_required {
        return if snapshot.expected_parent.is_none() {
            Ok(())
        } else {
            Err(RuntimeGitAuthorityError::IdentityMismatch)
        };
    }
    let parents: Vec<(Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT target_repository_id,target_commit FROM public.run_instance_provenance WHERE run_id=$1 LIMIT 2")
        .bind(run).fetch_all(&mut **tx).await.map_err(storage)?;
    let [parent] = parents.as_slice() else {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    };
    if parent.0 != Some(snapshot.repository_id)
        || parent.1 != snapshot.expected_parent
        || !snapshot.expected_parent.as_ref().is_some_and(|value| {
            matches!(value.len(), 40 | 64)
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(RuntimeGitAuthorityError::IdentityMismatch);
    }
    Ok(())
}
