//! Held projections never qualify another family or prospective admission.

use super::{Invocation, InvocationHold, Ordinary, OrdinaryHold, select};

#[test]
fn every_pair_of_held_reasons_stays_held() {
    let invocation = [
        InvocationHold::WorkerRoleRequired,
        InvocationHold::MissingRun,
        InvocationHold::UnsupportedProfile,
        InvocationHold::UnsupportedProducer,
        InvocationHold::ContradictoryOriginal,
        InvocationHold::ForeignOwner,
        InvocationHold::ConfigurationMismatch,
    ];
    let ordinary = [
        OrdinaryHold::WorkerRoleRequired,
        OrdinaryHold::MissingRequest,
        OrdinaryHold::ContradictoryOriginal,
        OrdinaryHold::PartialUnbornLineage,
        OrdinaryHold::UnsupportedProfile,
        OrdinaryHold::CanonicalOriginalFacadeRequired,
        OrdinaryHold::UnsupportedProducer,
        OrdinaryHold::ForeignOwner,
        OrdinaryHold::UnconsumedOriginal,
    ];
    for invocation_reason in invocation {
        for ordinary_reason in ordinary {
            assert!(
                select(
                    &Invocation::Held(invocation_reason),
                    &Ordinary::Held(ordinary_reason),
                )
                .is_err()
            );
        }
    }
}
