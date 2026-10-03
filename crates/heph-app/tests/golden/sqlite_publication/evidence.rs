use crate::*;
use std::fmt::Write;

fn digest_hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").unwrap();
    }
    result
}

pub async fn load_image(pool: &sqlx::PgPool, reference: &str) {
    let output = PathBuf::from(env::var("HEPHAESTUS_SQLITE_PUBLICATION_OUTPUT").unwrap());
    let input: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("image-input.json")).unwrap()).unwrap();
    let entry = &input["catalog_entry"];
    assert_eq!(entry["image_reference"], reference);
    let architectures = entry["architectures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    sqlx::query("INSERT INTO oci_images(id,key,display_name,image_reference,toolchains,architectures,availability_state,provenance,platform_policy_version) VALUES($1,'python-ubuntu',$2,$3,$4,$5,'available',$6,$7)")
        .bind(entry["id"].as_str().unwrap().parse::<uuid::Uuid>().unwrap())
        .bind(entry["display_name"].as_str().unwrap()).bind(reference)
        .bind(&entry["toolchains"]).bind(architectures).bind(&entry["provenance"])
        .bind(entry["platform_policy_version"].as_str().unwrap())
        .execute(pool).await.expect("exact reviewed catalog metadata");
}

pub fn token(user: UserId, session: &BrowserSessionSid, audience: &str) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss":"hephaestus-web-mediator", "sub":user.to_string(), "aud":audience,
            "iat":now,"nbf":now,"exp":now+25,"jti":uuid::Uuid::new_v4().to_string(),
            "sid":session.to_protocol_string(),
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("actual method-bound mediator token")
}

// Later recipe fixtures must rebuild/publish in their own live backend. Saved
// IDs from this disposable proof cannot replace authoritative catalog rows.
pub async fn build_and_publish(
    context: &cooking_builds::CookingBuildContext<'_>,
    source: &Path,
    artifacts: &Path,
) -> cooking_builds::PublishedCookingRepository {
    let published = cooking_builds::build_one(
        context,
        "local-sqlite",
        source.to_owned(),
        "Actual local SQLite",
        false,
    )
    .await
    .expect("real source build and publication");
    verify(context, &published, source, artifacts).await;
    published
}

pub async fn verify(
    context: &cooking_builds::CookingBuildContext<'_>,
    published: &cooking_builds::PublishedCookingRepository,
    source: &Path,
    artifacts: &Path,
) {
    let agent: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM release_agents WHERE release_id=$1 AND agent_key='local-sqlite'",
    )
    .bind(published.release_id)
    .fetch_one(context.pool)
    .await
    .expect("exact exported key");
    assert_eq!(agent, published.release_agent_id);
    let row: (uuid::Uuid, Vec<u8>, i64) = sqlx::query_as("SELECT storage_key,content_hash,size_bytes FROM release_artifacts WHERE release_id=$1 AND path='bin/local-sqlite' AND kind='executable'")
        .bind(published.release_id).fetch_one(context.pool).await.expect("real imported executable");
    let bytes =
        fs::read(artifacts.join(row.0.simple().to_string())).expect("canonical imported bytes");
    assert_eq!(bytes, fs::read(source.join("app.py")).unwrap());
    assert_eq!(Sha256::digest(&bytes).to_vec(), row.1);
    assert_eq!(i64::try_from(bytes.len()).unwrap(), row.2);
    let pins: (String, String, String, Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as("SELECT build.state,execution.state,release.state,release.build_definition_hash,release.configuration_hash,release.manifest_hash FROM build_requests build JOIN build_executions execution ON execution.build_request_id=build.id JOIN releases release ON release.build_request_id=build.id WHERE build.id=$1 AND release.id=$2 AND release.source_commit=$3")
        .bind(published.build_request_id).bind(published.release_id).bind(&published.source_commit).fetch_one(context.pool).await.unwrap();
    assert_eq!(
        (&*pins.0, &*pins.1, &*pins.2),
        ("succeeded", "drafted", "published")
    );
    assert!(pins.3.len() == 32 && pins.4.len() == 32 && pins.5.len() == 32);
    assert_eq!(digest_hex(&pins.3), published.build_definition_hash);
    assert_eq!(digest_hex(&pins.4), published.configuration_hash);
    assert_eq!(digest_hex(&pins.5), published.manifest_hash);
    let audience = "/hephaestus.release.v1.ReleaseService/GetRelease";
    let response = reqwest::Client::new()
        .post(format!("http://{}{audience}", context.running.http_addr()))
        .bearer_auth((context.identity.rpc_token)(audience))
        .header("connect-protocol-version", "1")
        .json(&serde_json::json!({"releaseId":{"value":published.release_id.to_string()}}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&response).expect("actual GetRelease response");
    let release = &body["release"];
    assert_eq!(release["state"], "RELEASE_STATE_PUBLISHED");
    assert_eq!(release["id"]["value"], published.release_id.to_string());
    let exports = release["agents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|agent| agent["agentKey"] == "local-sqlite")
        .collect::<Vec<_>>();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0]["id"]["value"], agent.to_string());
    let output = PathBuf::from(
        env::var("HEPHAESTUS_SQLITE_PUBLICATION_OUTPUT").expect("durable proof destination"),
    );
    fs::write(output.join("publication.json"), &response).expect("save actual response");
    fs::write(output.join("local-sqlite"), &bytes).expect("retain actual imported executable");
    fs::copy(source.join("agent.toml"), output.join("agent.toml")).unwrap();
    fs::copy(source.join("build.sh"), output.join("build.sh")).unwrap();
    let proof = serde_json::json!({"repository_id":published.repository_id.to_string(),"source_commit":published.source_commit,"build_request_id":published.build_request_id,"release_id":published.release_id,"release_agent_id":agent,"artifact_sha256":digest_hex(&row.1),"build_definition_hash":published.build_definition_hash,"configuration_hash":published.configuration_hash,"manifest_hash":published.manifest_hash,"image_reference":env::var("HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE").unwrap(),"runtime_protocol":11});
    fs::write(
        output.join("build-proof.json"),
        serde_json::to_vec_pretty(&proof).unwrap(),
    )
    .unwrap();
    println!("REAL_SQLITE_PUBLISHED {proof}");
}
