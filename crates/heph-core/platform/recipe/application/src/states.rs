use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::DeploymentError;

macro_rules! vocabulary {
    ($name:ident, $doc:literal, {$($variant:ident => ($wire:literal, $meaning:literal)),+ $(,)?}) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $(#[doc = $meaning] $variant,)+
        }
        impl $name {
            /// Returns the exact persisted closed-vocabulary value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire,)+ }
            }
        }
        impl FromStr for $name {
            type Err = DeploymentError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value { $($wire => Ok(Self::$variant),)+ _ => Err(DeploymentError::UnknownState) }
            }
        }
    };
}

vocabulary!(DeploymentLifecycle, "Durable deployment lifecycle, separate from per-resource progress.", {
    Installing => ("installing", "Install has admitted durable intent."),
    Installed => ("installed", "All required resources are ready."),
    Removing => ("removing", "Removal is draining and releasing consumers."),
    Removed => ("removed", "Owned resources are safely retained or deleted."),
    RecoveryRequired => ("recovery_required", "A provider outcome needs authoritative reconciliation.")
});
vocabulary!(InstallProgress, "Installation progress for one named resource.", {
    Pending => ("pending", "No creation or external verification has been claimed."),
    Creating => ("creating", "A durable attempt precedes a provider effect."),
    Ready => ("ready", "Exact resource identity and desired intent were verified."),
    Failed => ("failed", "A definite failure is inspectable."),
    RecoveryRequired => ("recovery_required", "An ambiguous result blocks another creation.")
});
vocabulary!(RemovalProgress, "Removal progress without conflating retention with deletion.", {
    Pending => ("pending", "Removal has not claimed this resource."),
    Draining => ("draining", "Consumers are being fenced and stopped."),
    Detached => ("detached", "Provider evidence proves prior attachments are released."),
    Retained => ("retained", "Stable resource ownership and data remain discoverable."),
    Deleted => ("deleted", "Authorized owned-resource deletion was confirmed."),
    RecoveryRequired => ("recovery_required", "Removal safety or its outcome is ambiguous.")
});
vocabulary!(ResourceAction, "Closed provider-independent resource effect vocabulary.", {
    Create => ("create", "Create one deployment-owned identity."),
    VerifyExternal => ("verify_external", "Verify an external identity without mutating it."),
    Drain => ("drain", "Close admission and drain an owned consumer."),
    Detach => ("detach", "Confirm removal of owned-resource attachments."),
    Retain => ("retain", "Record discoverable retained ownership."),
    Delete => ("delete", "Delete an owned resource after safety checks.")
});
vocabulary!(DeploymentOperation, "Caller-visible durable deployment commands.", {
    Install => ("install", "Admit or resume immutable deployment intent."),
    Remove => ("remove", "Admit or resume policy-bounded removal.")
});
vocabulary!(DiagnosticCode, "Closed non-sensitive progress diagnostics without provider error text.", {
    ProviderFailure => ("provider_failure", "The provider reported a definite failure."),
    ProviderOutcomeUnknown => ("provider_outcome_unknown", "Creation or removal may have succeeded."),
    DependencyUnavailable => ("dependency_unavailable", "An exact prerequisite is unavailable."),
    AuthorizationChanged => ("authorization_changed", "Live authority no longer permits this effect."),
    DrainPending => ("drain_pending", "Consumers have not proven safe shutdown."),
    FencingUnproven => ("fencing_unproven", "Prior attachment fencing remains unproven."),
    IntentConflict => ("intent_conflict", "Observed resource differs from immutable intent.")
});
