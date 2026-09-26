//! Composite organization cursor pagination against disposable `PostgreSQL`.

use control_plane_postgres::{
    connect_app,
    organization::{OrganizationApplication, OrganizationPage},
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::test]
async fn organization_pages_follow_name_id_cursor_across_insertion() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping organization pagination: test URL is unset");
        return;
    };

    let bootstrap = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL bootstrap pool");
    sqlx::migrate!("../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations");

    let owner = UserId::new();
    let name = format!("organization-pagination-{owner}");
    let id_base = Uuid::new_v4().as_u128() & !0xff;
    let first_id = Uuid::from_u128(id_base + 10);
    let second_id = Uuid::from_u128(id_base + 20);
    let inserted_id = Uuid::from_u128(id_base + 30);
    let final_id = Uuid::from_u128(id_base + 40);
    seed_owner_and_organizations(&bootstrap, owner, &name, [first_id, second_id, final_id]).await;

    let app_pool = connect_app(&database_url, 2)
        .await
        .expect("connect application-role pool");
    let application = OrganizationApplication::new(app_pool.clone());
    let identity = AuthenticatedIdentity::new(
        owner,
        "https://organization-pagination.example",
        format!("organization-pagination-{owner}"),
        json!({"email_verified": true}),
        RequestId::new(),
    );

    let first_page = application
        .list_organizations(
            &identity,
            OrganizationPage {
                size: 2,
                after: None,
            },
        )
        .await
        .expect("first organization page");
    assert_eq!(
        organization_ids(&first_page.organizations),
        [first_id, second_id]
    );
    let expected_cursor = second_id.to_string();
    assert_eq!(
        first_page.next_page_token.as_deref(),
        Some(expected_cursor.as_str())
    );

    // This insertion commits after page one and before page two, like a
    // concurrent writer. Its UUID is between the page cursor and final row.
    insert_organization(&bootstrap, inserted_id, &name, owner).await;

    let second_page = application
        .list_organizations(
            &identity,
            OrganizationPage {
                size: 2,
                after: Some(second_id),
            },
        )
        .await
        .expect("second organization page");
    assert_eq!(
        organization_ids(&second_page.organizations),
        [inserted_id, final_id]
    );
    assert_eq!(second_page.next_page_token, None);

    let third_page = application
        .list_organizations(
            &identity,
            OrganizationPage {
                size: 2,
                after: Some(final_id),
            },
        )
        .await
        .expect("terminal organization page");
    assert!(third_page.organizations.is_empty());
    assert_eq!(third_page.next_page_token, None);

    let paged_ids = first_page
        .organizations
        .into_iter()
        .chain(second_page.organizations)
        .map(|organization| organization.id)
        .collect::<Vec<_>>();
    assert_eq!(paged_ids, [first_id, second_id, inserted_id, final_id]);
    assert!(paged_ids.windows(2).all(|window| window[0] < window[1]));

    app_pool.close().await;
    bootstrap.close().await;
}

async fn seed_owner_and_organizations(
    pool: &sqlx::PgPool,
    owner: UserId,
    name: &str,
    organization_ids: [Uuid; 3],
) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(owner.as_uuid())
        .bind(format!("organization-pagination-{owner}"))
        .execute(pool)
        .await
        .expect("seed pagination user");
    for organization_id in organization_ids {
        insert_organization(pool, organization_id, name, owner).await;
    }
}

async fn insert_organization(pool: &sqlx::PgPool, id: Uuid, name: &str, owner: UserId) {
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await
        .expect("seed pagination organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(id)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed pagination membership");
}

fn organization_ids(
    organizations: &[control_plane_postgres::organization::OrganizationSummary],
) -> [Uuid; 2] {
    [organizations[0].id, organizations[1].id]
}
