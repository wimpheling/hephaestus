//! Actual protected completion retains first-Ready filesystem custody.

use super::bootstrap::Bootstrap;

pub async fn complete_owned(
    bootstrap: &Bootstrap,
    claim: recipe_application::EffectClaim,
    mut custody: volume_local::OwnedReadyCustody,
) {
    custody.revalidate().unwrap();
    let repository = bootstrap.recipe.clone();
    let actor = bootstrap.actor.clone();
    let worker = bootstrap.worker.clone();
    // Protected completion owns the non-clone native guard through the actual
    // worker record and actor commit, even if the calling fixture is cancelled.
    tokio::spawn(async move {
        custody.revalidate().unwrap();
        recipe_postgres::PostgresEffectVerificationRecorder::new(worker)
            .record(
                &claim,
                claim.provenance.attempt_id,
                recipe_postgres::VerifiedOutcome::Applied,
            )
            .await
            .unwrap();
        custody.revalidate().unwrap();
        repository
            .complete_resource_effect(
                &actor,
                &claim,
                recipe_application::EffectEvidence {
                    identity: claim.identity,
                    input_hash: claim.input_hash,
                    outcome: recipe_application::EffectOutcome::Applied,
                },
            )
            .await
            .unwrap();
        custody.revalidate().unwrap();
    })
    .await
    .unwrap();
}
