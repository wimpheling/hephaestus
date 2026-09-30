use authz_domain::{ObjectType, Permission};
use identity_domain::UserId;
use uuid::Uuid;

use super::support::Fixture;

#[derive(Clone, Copy)]
pub struct ExpectedCheck {
    pub subject: UserId,
    pub permission: Permission,
    pub object_type: ObjectType,
    pub object_id: Uuid,
    pub allowed: bool,
}

// The table is kept as one explicit parity matrix so each expected authorization is reviewable.
#[allow(clippy::too_many_lines)]
pub fn parity_checks(fixture: &Fixture) -> Vec<ExpectedCheck> {
    let mut checks = Vec::with_capacity(77);
    let mut add = |subject, object_type, object_id, assertions: &[(Permission, bool)]| {
        checks.extend(
            assertions
                .iter()
                .copied()
                .map(|(permission, allowed)| ExpectedCheck {
                    subject,
                    permission,
                    object_type,
                    object_id,
                    allowed,
                }),
        );
    };

    add(
        fixture.owner,
        ObjectType::Organization,
        fixture.organization,
        &[
            (Permission::CanRead, true),
            (Permission::CanManageMembers, true),
            (Permission::CanCreateProject, true),
        ],
    );
    add(
        fixture.admin,
        ObjectType::Organization,
        fixture.organization,
        &[
            (Permission::CanRead, true),
            (Permission::CanManageMembers, true),
            (Permission::CanCreateProject, true),
            (Permission::CanDelete, false),
        ],
    );
    add(
        fixture.member,
        ObjectType::Organization,
        fixture.organization,
        &[
            (Permission::CanRead, true),
            (Permission::CanManageMembers, false),
            (Permission::CanCreateProject, false),
        ],
    );
    add(
        fixture.outsider,
        ObjectType::Organization,
        fixture.organization,
        &[(Permission::CanRead, false)],
    );
    add(
        fixture.member,
        ObjectType::Project,
        fixture.project,
        &[(Permission::CanRead, true), (Permission::CanWrite, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Project,
        fixture.project,
        &[
            (Permission::CanRead, true),
            (Permission::CanWrite, true),
            (Permission::CanManage, true),
            (Permission::CanDelete, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::Repository,
        fixture.private_repository,
        &[
            (Permission::CanRead, true),
            (Permission::CanWrite, true),
            (Permission::CanDelete, false),
        ],
    );
    add(
        fixture.owner,
        ObjectType::Repository,
        fixture.private_repository,
        &[
            (Permission::CanRead, true),
            (Permission::CanWrite, false),
            (Permission::CanDelete, true),
        ],
    );
    add(
        fixture.outsider,
        ObjectType::Repository,
        fixture.private_repository,
        &[(Permission::CanRead, false)],
    );
    add(
        fixture.outsider,
        ObjectType::Repository,
        fixture.public_repository,
        &[(Permission::CanRead, true), (Permission::CanWrite, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Build,
        fixture.build,
        &[
            (Permission::CanRead, true),
            (Permission::CanExecute, true),
            (Permission::CanCancel, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::Build,
        fixture.build,
        &[(Permission::CanRead, true), (Permission::CanExecute, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Release,
        fixture.release,
        &[
            (Permission::CanRead, true),
            (Permission::CanPublish, true),
            (Permission::CanRevoke, true),
            (Permission::CanUse, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::Release,
        fixture.release,
        &[
            (Permission::CanRead, true),
            (Permission::CanPublish, false),
            (Permission::CanRevoke, false),
            (Permission::CanUse, true),
        ],
    );
    add(
        fixture.outsider,
        ObjectType::Release,
        fixture.release,
        &[
            (Permission::CanRead, false),
            (Permission::CanRevoke, false),
            (Permission::CanUse, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::ReleaseAgent,
        fixture.release_agent,
        &[(Permission::CanRead, true), (Permission::CanUse, true)],
    );
    add(
        fixture.outsider,
        ObjectType::ReleaseAgent,
        fixture.release_agent,
        &[(Permission::CanRead, false), (Permission::CanUse, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::AgentInstance,
        fixture.instance,
        &[
            (Permission::CanRead, true),
            (Permission::CanExecute, true),
            (Permission::CanManage, true),
            (Permission::CanUpdate, true),
            (Permission::CanRecover, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::AgentInstance,
        fixture.instance,
        &[
            (Permission::CanRead, true),
            (Permission::CanExecute, false),
            (Permission::CanUpdate, false),
            (Permission::CanRecover, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::AgentAttachment,
        fixture.attachment,
        &[
            (Permission::CanRead, true),
            (Permission::CanManage, true),
            (Permission::CanExecute, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::AgentAttachment,
        fixture.attachment,
        &[
            (Permission::CanRead, true),
            (Permission::CanManage, false),
            (Permission::CanExecute, false),
        ],
    );
    add(
        fixture.maintainer,
        ObjectType::AgentUpdate,
        fixture.update,
        &[(Permission::CanRead, true), (Permission::CanRecover, true)],
    );
    add(
        fixture.member,
        ObjectType::AgentUpdate,
        fixture.update,
        &[(Permission::CanRead, true), (Permission::CanRecover, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::Run,
        fixture.run,
        &[(Permission::CanRead, true), (Permission::CanCancel, true)],
    );
    add(
        fixture.member,
        ObjectType::Run,
        fixture.run,
        &[(Permission::CanRead, true), (Permission::CanCancel, false)],
    );
    add(
        fixture.maintainer,
        ObjectType::StateVolume,
        fixture.volume,
        &[
            (Permission::CanRead, true),
            (Permission::CanAttach, true),
            (Permission::CanRestore, true),
            (Permission::CanManage, true),
        ],
    );
    add(
        fixture.member,
        ObjectType::StateVolume,
        fixture.volume,
        &[
            (Permission::CanRead, true),
            (Permission::CanAttach, false),
            (Permission::CanRestore, false),
        ],
    );
    add(
        fixture.revoked,
        ObjectType::Project,
        fixture.project,
        &[(Permission::CanRead, false), (Permission::CanWrite, false)],
    );
    add(
        fixture.revoked,
        ObjectType::Repository,
        fixture.private_repository,
        &[(Permission::CanRead, false), (Permission::CanWrite, false)],
    );

    assert_eq!(checks.len(), 80, "fixture must match OpenFGA checks");
    checks
}
