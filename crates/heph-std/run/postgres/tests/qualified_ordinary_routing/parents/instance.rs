use super::Parents;
use sqlx::PgPool;

pub(super) async fn seed(pool: &PgPool, p: &Parents) {
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state) VALUES($1,$2,$3,'ordinary','active')")
        .bind(p.instance).bind(p.repository.project_id.as_uuid()).bind(p.family).execute(pool).await.expect("parent legacy instance");
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) VALUES($1,$2,$3,'{}',$4,'{}','{}','{}',$5,'test/v1',true)")
        .bind(p.revision).bind(p.instance).bind(p.agent).bind([5_u8;32].as_slice()).bind([6_u8;32].as_slice()).execute(pool).await.expect("parent immutable revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(p.instance)
        .bind(p.revision)
        .execute(pool)
        .await
        .expect("active revision");
    sqlx::query("INSERT INTO agent_attachments(id,instance_id,project_id,repository_id,ref_selector,trigger_policy) VALUES($1,$2,$3,$4,'refs/heads/main','push')")
        .bind(p.attachment).bind(p.instance).bind(p.repository.project_id.as_uuid()).bind(p.repository.id.as_uuid()).execute(pool).await.expect("actual push attachment");
}
