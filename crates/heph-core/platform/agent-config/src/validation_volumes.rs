//! Named-volume catalog validation without consumer grants.

use crate::{AgentConfig, Diagnostic, validation_shared::diagnostic};

pub fn validate_volume_slots(config: &AgentConfig, diagnostics: &mut Vec<Diagnostic>) {
    if let Err(error) = config.effective_volume_slots() {
        diagnostic(
            diagnostics,
            "invalid_volume_slots",
            "volume_slots",
            error.to_string(),
        );
    }
}
