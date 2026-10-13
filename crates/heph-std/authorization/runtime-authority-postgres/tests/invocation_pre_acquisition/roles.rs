use super::fixture::{Fixture, code};
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn worker_only_boolean_boundary_preserves_private_helper_acls(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let before = f.rows().await;
    for role in [&f.birth.birth.app, &pool] {
        let configured = PgRuntimeSessionRepository::new(role.clone())
            .with_qualified_invocation_configuration(f.birth.config.clone());
        assert!(
            configured
                .verify_qualified_invocation_pre_acquisition(&f.run, AUTHORIZATION_MODEL_VERSION)
                .await
                .is_err()
        );
        let error = f
            .raw(role, &f.run, &f.config(), AUTHORIZATION_MODEL_VERSION)
            .await
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("42501"));
    }
    let properties:(bool,bool,bool,bool,bool)=sqlx::query_as("SELECT p.prosecdef,NOT p.proretset,p.prorettype='boolean'::regtype,EXISTS(SELECT 1 FROM unnest(p.proconfig) c WHERE c='search_path=pg_catalog, public, pg_temp'),NOT EXISTS(SELECT 1 FROM aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE a.grantee=0 AND a.privilege_type='EXECUTE') FROM pg_proc p WHERE p.oid='verify_invocation119_pre_acquisition(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,bigint,text)'::regprocedure")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(properties, (true, true, true, true, true));
    let acl:(bool,bool)=sqlx::query_as("SELECT has_function_privilege('hephaestus_worker','verify_invocation119_pre_acquisition(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,bigint,text)','EXECUTE'),has_function_privilege('hephaestus_app','verify_invocation119_pre_acquisition(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,bigint,text)','EXECUTE')")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(acl, (true, false));
    //119-A grants only these metadata readers to the worker. B1 keeps its
    // capability helper revoked; the new facade must preserve both ACL shapes.
    for (helper, worker_execute) in [
        ("invocation119_immutable(uuid)", true),
        ("invocation119_open_set(uuid)", true),
        ("invocation119_live(uuid)", true),
        ("invocation119_frozen_set(uuid)", true),
        ("invocation119_capabilities_live(uuid)", false),
    ] {
        let acl:(bool,bool)=sqlx::query_as("SELECT has_function_privilege('hephaestus_worker',$1,'EXECUTE'),has_function_privilege('hephaestus_app',$1,'EXECUTE')")
            .bind(helper).fetch_one(&pool).await.unwrap();
        assert_eq!(acl, (worker_execute, false), "{helper}");
    }
    assert_eq!(f.rows().await, before);
    f.allowed().await;
}
