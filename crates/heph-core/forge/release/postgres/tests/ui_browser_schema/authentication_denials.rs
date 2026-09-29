use super::*;

// Keep the negative route, generation, account, and installation cases in
// one phase so every denial is checked against the same seeded child.
#[allow(
    clippy::cognitive_complexity,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::too_many_lines
)]
pub async fn assert_authentication_denials(ctx: &AuthenticationContext) {
    let worker = ctx.worker.clone();
    let app = ctx.app.clone();
    let fixture = &ctx.fixture;
    let store = &ctx.store;
    let child_digest = &ctx.child_digest;
    for route in ["schema-ui/missing.js", "other-ui/index.html"] {
        let denied = store
            .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
                session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
                expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
                request_route: UiBrowserRequestRoute::Static {
                    route: UiBrowserRoute::parse(route).expect("safe negative route"),
                },
            })
            .await;
        assert_eq!(denied, Err(UiBrowserSessionError::Unauthenticated));
    }
    let wrong_generation = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(
        wrong_generation,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    let wrong_kind = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Managed {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(wrong_kind, Err(UiBrowserSessionError::Unauthenticated));
    let wrong_method = sqlx::query(
        "SELECT session_id FROM authenticate_ui_browser_session(
         $1, $2, 'static', 'schema-ui', 'POST'
     )",
    )
    .bind(&child_digest)
    .bind(fixture.generation)
    .fetch_optional(&app)
    .await
    .expect("wrong method returns zero rows");
    assert!(wrong_method.is_none());

    let expired_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 91))
        .digest()
        .as_bytes()
        .to_vec();
    insert_authenticated_child(&worker, &fixture, expired_digest, "1 second").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let expired = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 91)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(expired, Err(UiBrowserSessionError::Unauthenticated));

    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("suspend account");
    let suspended = store
        .authenticate_ui_browser_session(AuthenticateUiBrowserSession {
            session_secret: UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 90)),
            expected_generation_id: UiInstallationGenerationId::from_uuid(fixture.generation),
            request_route: UiBrowserRequestRoute::Static {
                route: UiBrowserRoute::parse("schema-ui").expect("route base"),
            },
        })
        .await;
    assert_eq!(suspended, Err(UiBrowserSessionError::Unauthenticated));
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore account");

    let replacement_generation = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ui_installation_generations
     (id, installation_id, generation_no, release_id, ui_key, ui_scope)
     VALUES ($1, $2, 2, $3, 'schema-ui', 'project')",
    )
    .bind(replacement_generation)
    .bind(fixture.installation)
    .bind(fixture.release)
    .execute(&worker)
    .await
    .expect("insert replacement generation");
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(replacement_generation)
        .execute(&worker)
        .await
        .expect("move installation current generation");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(fixture.generation)
        .execute(&worker)
        .await
        .expect("restore installation current generation");

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("disable installation");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.installation)
        .execute(&worker)
        .await
        .expect("restore installation lifecycle");

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke target project maintainer");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke target organization membership");
    assert_eq!(
        authenticate_static(&store, &fixture, test_secret(fixture.actor, 90)).await,
        Err(UiBrowserSessionError::Unauthenticated)
    );
    sqlx::query("INSERT INTO organization_members (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore target organization membership");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(fixture.project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("restore target project maintainer");
    let initial_release_agent_use: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_use', 'release_agent', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.release_agent.to_string())
            .fetch_one(&worker)
            .await
            .expect("check source release-agent use before revocation");
    assert_eq!(initial_release_agent_use, 1);

    // The release is published from source_project while the browser installs
    // live in the target project. Removing only source authority must deny
    // every source-backed binding and keep the explicit target read grant.
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(fixture.source_project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke source project maintainer");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke source organization membership");
    let target_read: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_read', 'project', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.project.to_string())
            .fetch_one(&worker)
            .await
            .expect("check target project read after source revocation");
    assert_eq!(
        target_read, 1,
        "target project read survives source revocation"
    );
    let source_read: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_read', 'project', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.source_project.to_string())
            .fetch_one(&worker)
            .await
            .expect("check source project read after revocation");
    assert_eq!(source_read, 0);
    let release_agent_use: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_use', 'release_agent', $2)")
            .bind(fixture.actor.to_string())
            .bind(fixture.release_agent.to_string())
            .fetch_one(&worker)
            .await
            .expect("check source release-agent use after revocation");
    assert_eq!(release_agent_use, 0);
}
