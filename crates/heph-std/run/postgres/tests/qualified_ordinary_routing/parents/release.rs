use super::Parents;
use forge_domain::CommitSha;
use sqlx::PgPool;
use uuid::Uuid;

pub(super) async fn seed(pool: &PgPool, p: &Parents, commit: &CommitSha) {
    let organization = Uuid::new_v4();
    let build = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations(id,name) VALUES($1,$2)")
        .bind(organization)
        .bind(format!("ordinary-{organization}"))
        .execute(pool)
        .await
        .expect("parent organization");
    sqlx::query("INSERT INTO projects(id,organization_id,name) VALUES($1,$2,'ordinary')")
        .bind(p.repository.project_id.as_uuid())
        .bind(organization)
        .execute(pool)
        .await
        .expect("parent project");
    sqlx::query("INSERT INTO repositories(id,project_id,name) VALUES($1,$2,'ordinary-routing')")
        .bind(p.repository.id.as_uuid())
        .bind(p.repository.project_id.as_uuid())
        .execute(pool)
        .await
        .expect("parent repository");
    sqlx::query("INSERT INTO agent_families(id,repository_id,agent_key) VALUES($1,$2,'runtime')")
        .bind(p.family)
        .bind(p.repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("parent family");
    sqlx::query("INSERT INTO build_requests(id,repository_id,source_commit,source_ref,build_definition_hash,state,completed_at) VALUES($1,$2,$3,'refs/heads/main',$4,'succeeded',now())")
        .bind(build).bind(p.repository.id.as_uuid()).bind(commit.as_str()).bind([1_u8;32].as_slice()).execute(pool).await.expect("parent successful build");
    sqlx::query("INSERT INTO releases(id,repository_id,version,source_commit,source_ref,build_request_id,build_definition_hash,configuration,configuration_hash,manifest_hash,state,published_at) VALUES($1,$2,'ordinary-v1',$3,'refs/heads/main',$4,$5,'{}',$6,$7,'published',now())")
        .bind(p.release).bind(p.repository.id.as_uuid()).bind(commit.as_str()).bind(build).bind([1_u8;32].as_slice()).bind([2_u8;32].as_slice()).bind([3_u8;32].as_slice()).execute(pool).await.expect("parent published release");
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) VALUES($1,$2,$3,'runtime','Runtime','{}',$4,false)")
        .bind(p.agent).bind(p.release).bind(p.family).bind([4_u8;32].as_slice()).execute(pool).await.expect("parent stateless release agent");
}
